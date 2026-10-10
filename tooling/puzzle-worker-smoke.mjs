import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { mkdtemp, rm } from 'node:fs/promises'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { zstdCompressSync } from 'node:zlib'

// The Rust core's puzzle service through the built @kchess/native module (the same binding
// core/src/services/native.ts loads), an isolated data directory and a deterministic local
// download. Run after build.
const native = createRequire(new URL('../core/src/services/native.ts', import.meta.url))(
  '@kchess/native',
)
const directory = await mkdtemp(join(tmpdir(), 'kchess-puzzle-service-'))
const header = 'PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes\n'
const fen = '6k1/P6p/8/8/8/8/5PPP/6K1 b - - 0 1'
const csv =
  header +
  Array.from(
    { length: 100_000 },
    (_, index) => `p${index},${fen},h7h6 a7a8q,${500 + (index % 3000)},50,100,1000,promotion`,
  ).join('\n')
let payload = zstdCompressSync(csv)
const server = createServer((_request, response) => response.end(payload))
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
const address = server.address()
assert(address && typeof address === 'object')

const phases = []
const warnings = []
let cancelOnImport = false
let cancelError
const core = new native.NativeCore(
  { dataDir: directory, legacyDatabasePath: join(directory, 'missing.db') },
  (json) => {
    const line = JSON.parse(json)
    if (line.level === 'warn' || line.level === 'error') warnings.push(line.message)
  },
  (json) => {
    const { event, payload: progress } = JSON.parse(json)
    if (event !== 'puzzledb:progress') return
    phases.push(progress.phase)
    if (progress.phase === 'importing' && cancelOnImport) {
      cancelOnImport = false
      call('puzzles.cancel').catch((cause) => {
        cancelError = cause
      })
    }
  },
)
function call(method, ...args) {
  let timer
  const timeout = new Promise((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`Puzzle service ${method} timed out.`)), 30_000)
  })
  const result = core.call(method, JSON.stringify(args)).then((json) => JSON.parse(json))
  return Promise.race([result, timeout]).finally(() => clearTimeout(timer))
}
const url = `http://127.0.0.1:${address.port}/puzzles.zst`
let ticks = 0
let longestGap = 0
let lastTick = performance.now()
const heartbeat = setInterval(() => {
  const now = performance.now()
  longestGap = Math.max(longestGap, now - lastTick)
  lastTick = now
  ticks++
}, 10)
try {
  const started = performance.now()
  const installed = await call('puzzles.install', url)
  const elapsed = Math.round(performance.now() - started)
  assert.equal(installed.count, 100_000)
  for (const phase of ['downloading', 'importing', 'done'])
    assert(phases.includes(phase), `The ${phase} progress phase must arrive.`)
  assert(ticks > 5, 'The host heartbeat must keep running during import.')
  payload = zstdCompressSync(header + 'bad,invalid,invalid,1500,50,100,1000,promotion\n')
  await assert.rejects(call('puzzles.install', url), /usable/)
  assert.equal((await call('puzzles.status')).count, 100_000)
  payload = zstdCompressSync(csv)
  phases.length = 0
  cancelOnImport = true
  await call('puzzles.install', url)
  assert(phases.includes('cancelled'), 'Cancelling during the import must be reported.')
  assert.equal(cancelError, undefined)
  assert.equal((await call('puzzles.status')).importedAt, installed.importedAt)
  assert.equal((await call('puzzles.delete')).installed, false)
  console.info(
    JSON.stringify({
      rows: installed.count,
      importMs: elapsed,
      hostTicks: ticks,
      longestHostGapMs: Math.round(longestGap),
      rollback: 'passed',
      cancellation: 'passed',
    }),
  )
} finally {
  clearInterval(heartbeat)
  await core.close()
  await new Promise((resolve) => server.close(resolve))
  await rm(directory, { recursive: true, force: true })
  if (warnings.length) console.info(`Puzzle service warnings: ${warnings.join(' | ')}`)
}
