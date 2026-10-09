// Build first. These hosts use temporary profiles and never access the user's keychain or network.
import assert from 'node:assert/strict'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawn } from 'node:child_process'
import { DatabaseSync } from 'node:sqlite'
import { createNodeCore } from '../hosts/node/dist/node.js'
import { OnlineGame, onlineGameState } from '../core/dist/onlineGame.js'
import { PuzzleSession, puzzleSessionState } from '../core/dist/puzzleSession.js'

const dirs = [
  await mkdtemp(join(tmpdir(), 'kchess-node-a-')),
  await mkdtemp(join(tmpdir(), 'kchess-node-b-')),
]
const clients = []
function cli(args, input = '') {
  return new Promise((resolve, reject) => {
    const child = spawn(
      process.execPath,
      ['apps/cli/dist/cli.js', '--data-dir', dirs[1], '--no-credentials', ...args],
      { stdio: ['pipe', 'pipe', 'pipe'] },
    )
    const timeout = setTimeout(() => {
      child.kill()
      reject(new Error('CLI smoke timed out.'))
    }, 15000)
    let stdout = '',
      stderr = ''
    child.stdout.on('data', (data) => {
      stdout += data
    })
    child.stderr.on('data', (data) => {
      stderr += data
    })
    child.once('error', reject)
    child.once('exit', (code) => {
      clearTimeout(timeout)
      resolve({ code, stdout, stderr })
    })
    child.stdin.end(input)
  })
}
try {
  const [a, b] = await Promise.all(
    dirs.map((dataDir) => createNodeCore({ dataDir, disableCredentials: true })),
  )
  clients.push(a, b)
  const online = new OnlineGame(onlineGameState(), {
    api: b,
    activeAccount: () => '',
    chatEnabled: () => false,
    now: () => 0,
    failed: (cause) => {
      throw cause
    },
  })
  online.readOnlineState({ session: 1, account: '', gameId: '', lane: 'game', phase: 'idle' })
  assert.equal(online.state.onlinePhase, 'idle')
  const training = new PuzzleSession(puzzleSessionState(), {
    api: b,
    online: () => false,
    selection: () => ({
      account: '',
      mode: 'rated',
      angle: 'mix',
      difficulty: 'normal',
      color: 'random',
    }),
  })
  await training.loadNext()
  assert.equal(training.state.phase, 'noaccount')
  await a.addAccount('ProfileA')
  assert.deepEqual((await b.loadData()).accounts, [])
  const data = await a.loadData()
  const seen = []
  const off = a.on('settings:saved', (settings) => seen.push(settings.cloudEval))
  await a.saveSettings({ ...data.settings, cloudEval: true })
  off()
  await a.saveSettings(data.settings)
  assert.deepEqual(seen, [true])
  assert.equal((await b.loadData()).settings.cloudEval, false)
  await a.studyCommand({ op: 'save', name: 'Node', pgn: '1. e4 e5 *' })
  assert.equal((await a.library()).studies.length, 1)
  assert.equal((await b.library()).studies.length, 0)
  await Promise.all([a.resumeOnline(), b.resumeOnline()])
  const moves = await Promise.all([
    a.bestMove(['e2e4'], 'beginner'),
    b.bestMove(['d2d4'], 'beginner'),
  ])
  assert.ok(moves.every((move) => /^[a-h][1-8][a-h][1-8][qrbn]?$/.test(move)))
  assert.equal((await b.puzzleDbStatus()).installed, false)
  await assert.rejects(b.loadData('extra'), /argument count/)
  await a.close()
  await a.close()
  await assert.rejects(a.library(), /closed/)
  assert.equal((await b.library()).studies.length, 0)
  await b.close()
  const reopened = await createNodeCore({ dataDir: dirs[0], disableCredentials: true })
  clients.push(reopened)
  assert.deepEqual(
    (await reopened.loadData()).accounts.map((account) => account.username),
    ['ProfileA'],
  )
  // Offline document editing must not trigger account/live-game recovery.
  const fixture = new DatabaseSync(join(dirs[1], 'kchess.db'))
  fixture.prepare('INSERT INTO accounts (username, connected) VALUES (?, 1)').run('OfflineUser')
  fixture.close()
  const study = await cli([
    'call',
    'studyCommand',
    JSON.stringify([{ op: 'save', name: 'Offline', pgn: '1. e4 e5 *' }]),
  ])
  assert.equal(study.code, 0, study.stderr)
  assert.equal(JSON.parse(study.stdout).studies[0].name, 'Offline')
  const cleanup = new DatabaseSync(join(dirs[1], 'kchess.db'))
  cleanup.exec('DELETE FROM accounts')
  cleanup.close()
  const library = await cli(['library'])
  assert.equal(library.code, 0, library.stderr)
  assert.deepEqual(JSON.parse(library.stdout).games, [])
  const invalid = await cli(['call', 'loadData', '[1]'])
  assert.equal(invalid.code, 1)
  assert.equal(invalid.stdout, '')
  assert.match(invalid.stderr, /argument count/)
  const local = await cli(['local'], 'e2e4\ne7e5\nquit\n')
  assert.equal(local.code, 0, local.stderr)
  const saved = await cli(['library'])
  assert.deepEqual(JSON.parse(saved.stdout).sessions.local.moves, ['e2e4', 'e7e5'])
  const play = await cli(['play', 'beginner'], 'e2e4\nquit\n')
  assert.equal(play.code, 0, play.stderr)
  const computer = await cli(['library'])
  assert.equal(JSON.parse(computer.stdout).sessions.computer.moves.length, 2)
  console.info('ok   isolated Node hosts, events, engines, worker, persistence and CLI')
} finally {
  await Promise.allSettled(clients.map((core) => core.close()))
  await Promise.all(dirs.map((dir) => rm(dir, { recursive: true, force: true })))
}
