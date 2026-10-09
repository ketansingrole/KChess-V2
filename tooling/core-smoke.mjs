// The built headless core (core/dist) in plain Node: no Electron, no renderer.
import { mkdtemp, readdir, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const out = join(root, 'core/dist')
let failures = 0
function assert(name, actual, expected = true) {
  const ok = JSON.stringify(actual) === JSON.stringify(expected)
  if (!ok) failures++
  console.info(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : `: ${JSON.stringify(actual)}`}`)
}

const files = await readdir(out, { recursive: true })
const bundle = await Promise.all(
  files.filter((file) => file.endsWith('.js')).map((file) => readFile(join(out, file), 'utf8')),
)
assert(
  'the core bundle never loads Electron',
  bundle.some((code) => /from ["']electron["']|require\(["']electron["']\)/.test(code)),
  false,
)

const { createKChessCore, BUNDLED_ENGINE_SCRIPT, CORE_METHODS } = await import(
  join(out, 'index.js')
)
const dataDir = await mkdtemp(join(tmpdir(), 'kchess-core-smoke-'))
const host = (dataDir) => ({
  dataDir,
  bundledEnginePath: join(root, BUNDLED_ENGINE_SCRIPT),
  managedEngineDir: join(dataDir, 'engines'),
  // No OS keychain here: tokens are never stored.
  secrets: {
    available: () => false,
    encrypt: () => {
      throw new Error('unavailable')
    },
    decrypt: () => {
      throw new Error('unavailable')
    },
  },
  openExternal: async () => {},
  onBattery: () => false,
})
const secondDir = await mkdtemp(join(tmpdir(), 'kchess-core-second-'))
const core = createKChessCore(host(dataDir))
const second = createKChessCore(host(secondDir))

try {
  assert(
    'every CoreApi method is served',
    CORE_METHODS.every((m) => typeof core[m] === 'function'),
  )
  const data = await core.loadData()
  assert('a fresh profile has no accounts', data.accounts, [])
  const saved = await core.saveSettings({ ...data.settings, cloudEval: true })
  assert('settings persist', (await core.loadData()).settings.cloudEval, saved.cloudEval)
  await core.saveRun({ kind: 'storm', variant: '3min', score: 21, detail: {} })
  assert('runs persist', (await core.runSummary('storm')).best['3min']?.score, 21)
  const study = await core.studyCommand({ op: 'save', name: 'Smoke', pgn: '1. e4 e5 *' })
  assert(
    'studies persist',
    study.studies.map((s) => s.name),
    ['Smoke'],
  )
  await core.saveSession('analysis', {
    pgn: '1. e4 e5 *',
    path: 'e2e4',
    orientation: 'black',
    study: study.id,
    chapter: '',
  })
  assert('sessions persist', (await core.library()).sessions.analysis?.orientation, 'black')
  // Assistance stays off until startup recovery confirms no live Lichess game.
  assert('startup recovery finds no live game', await core.resumeOnline(), null)
  const engine = await core.engineStatus()
  assert('the bundled engine is ready', engine.ready && engine.bundled)
  const move = await core.bestMove(['e2e4'], 'beginner')
  assert('the bundled engine plays a move', /^[a-h][1-8][a-h][1-8][qrbn]?$/.test(move))
  assert('the puzzle worker answers', (await core.puzzleDbStatus()).installed, false)
  await second.resumeOnline()
  const secondMove = second.bestMove(['d2d4'], 'beginner')
  const firstMove = core.bestMove(['c2c4'], 'beginner')
  const moves = await Promise.all([firstMove, secondMove])
  assert(
    'direct cores search concurrently in one runtime',
    moves.every((move) => /^[a-h][1-8][a-h][1-8][qrbn]?$/.test(move)),
  )
  assert(
    'direct core puzzle workers are independent',
    (await second.puzzleDbStatus()).installed,
    false,
  )
  assert('direct core libraries are independent', (await second.library()).studies, [])
  assert(
    'headless settings omit frontend preferences',
    'boardTheme' in (await second.settings()),
    false,
  )
  await core.close()
  assert('closing one direct core leaves the other usable', (await second.engineStatus()).ready)
  assert(
    'remaining core still searches after sibling shutdown',
    /^[a-h][1-8][a-h][1-8][qrbn]?$/.test(await second.bestMove(['e2e4'], 'beginner')),
  )
} finally {
  await Promise.all([core.close(), second.close()])
  await rm(secondDir, { recursive: true, force: true })
  await rm(dataDir, { recursive: true, force: true })
}
if (failures) {
  console.error(`[kchess] core smoke: ${failures} failed`)
  process.exitCode = 1
}
