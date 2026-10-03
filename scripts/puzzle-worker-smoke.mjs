import assert from 'node:assert/strict'
import { mkdtemp, rm } from 'node:fs/promises'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Worker } from 'node:worker_threads'
import { zstdCompressSync } from 'node:zlib'

// Production worker, deterministic transport and isolated on-disk state. Run after build.
const directory = await mkdtemp(join(tmpdir(), 'kchess-puzzle-worker-'))
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
const worker = new Worker(new URL('../out/main/puzzleWorker.js', import.meta.url), {
  workerData: { path: join(directory, 'puzzles.db'), legacyPath: join(directory, 'missing.db') },
})
let next = 0
const waiting = new Map()
let cancelImport
worker.on('message', (message) => {
  if (message.progress?.phase === 'importing') cancelImport?.()
  const request = waiting.get(message.id)
  if (!request) return
  waiting.delete(message.id)
  clearTimeout(request.timer)
  if (message.error) request.reject(new Error(message.error))
  else request.resolve(message.result)
})
worker.on('error', (error) => {
  for (const entry of waiting.values()) {
    clearTimeout(entry.timer)
    entry.reject(error)
  }
  waiting.clear()
})
function call(method, options = {}) {
  const id = ++next
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`Worker ${method} timed out.`)), 30_000)
    waiting.set(id, { resolve, reject, timer })
    worker.postMessage({ id, method, ...options })
  })
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
  const installed = await call('install', { url, cancellation: new SharedArrayBuffer(4) })
  const elapsed = Math.round(performance.now() - started)
  assert.equal(installed.count, 100_000)
  assert(ticks > 5, 'The host heartbeat must keep running during import.')
  payload = zstdCompressSync(header + 'bad,invalid,invalid,1500,50,100,1000,promotion\n')
  await assert.rejects(call('install', { url, cancellation: new SharedArrayBuffer(4) }), /usable/)
  assert.equal((await call('status')).count, 100_000)
  payload = zstdCompressSync(csv)
  const cancellation = new Int32Array(new SharedArrayBuffer(4))
  cancelImport = () => Atomics.store(cancellation, 0, 1)
  await call('install', { url, cancellation: cancellation.buffer })
  assert.equal((await call('status')).importedAt, installed.importedAt)
  cancelImport = undefined
  assert.equal((await call('delete')).installed, false)
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
  await worker.terminate()
  await new Promise((resolve) => server.close(resolve))
  await rm(directory, { recursive: true, force: true })
}
