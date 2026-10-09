import { parentPort, workerData } from 'node:worker_threads'
import { DatabaseSync } from 'node:sqlite'
import { existsSync, chmodSync } from 'node:fs'
import { Readable } from 'node:stream'
import { sampleZstdCsv } from './puzzleSampler'
import { createPuzzleSampler } from './rules'
import { logDebug, logWarn } from './logger'
import { clearStored, queryLadder, queryPuzzles, readStatus, storeSample } from './puzzleQueries'
import type { LocalLadderQuery, LocalPuzzleQuery, PuzzleDbProgress } from '../contracts/types'

const data = workerData as { path: string; legacyPath: string }
const database = new DatabaseSync(data.path)
database.exec(`PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS puzzles (id TEXT PRIMARY KEY, fen TEXT NOT NULL, moves TEXT NOT NULL, rating INTEGER NOT NULL, plays INTEGER NOT NULL DEFAULT 0, themes TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS idx_puzzles_rating ON puzzles(rating);
CREATE TABLE IF NOT EXISTS puzzle_meta (id INTEGER PRIMARY KEY CHECK(id = 1), importedAt INTEGER NOT NULL, count INTEGER NOT NULL, bytes INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS worker_meta (id INTEGER PRIMARY KEY CHECK(id = 1));`)
try {
  chmodSync(data.path, 0o600)
} catch (cause) {
  logDebug('puzzles', 'Could not restrict database file permissions', cause)
}
// The old database remains an intact fallback; all subsequent puzzle writes have one worker owner.
if (
  !database.prepare('SELECT id FROM worker_meta WHERE id = 1').get() &&
  existsSync(data.legacyPath)
) {
  database.prepare('ATTACH DATABASE ? AS legacy').run(data.legacyPath)
  database.exec(`BEGIN;
    INSERT OR IGNORE INTO puzzles SELECT * FROM legacy.puzzles;
    INSERT OR IGNORE INTO puzzle_meta SELECT id, importedAt, count, bytes FROM legacy.puzzle_meta;
    INSERT OR REPLACE INTO worker_meta VALUES (1);
    COMMIT; DETACH DATABASE legacy;`)
}
database.exec('INSERT OR IGNORE INTO worker_meta VALUES (1)')
let running: AbortController | undefined
async function install(url: string, cancelled: Int32Array): Promise<unknown> {
  if (running) throw new Error('The puzzle database is already downloading.')
  const controller = new AbortController()
  running = controller
  const sampler = createPuzzleSampler()
  let received = 0
  let total: number | undefined
  let lastReport = 0
  const report = (phase: PuzzleDbProgress['phase'], force = false): void => {
    if (!force && Date.now() - lastReport < 250) return
    lastReport = Date.now()
    parentPort!.postMessage({ progress: { phase, received, total, kept: sampler.count } })
  }
  try {
    const response = await fetch(url, { signal: controller.signal })
    parentPort!.postMessage({ usage: { requests: 1, bytes: 0 } })
    if (!response.ok || !response.body)
      throw new Error(`Lichess answered ${response.status} for the puzzle database.`)
    total = Number(response.headers.get('content-length')) || undefined
    const source = Readable.fromWeb(response.body as never)
    source.on('data', (chunk: Buffer) => {
      received += chunk.byteLength
      report('downloading')
    })
    await sampleZstdCsv(source, sampler, controller.signal)
    controller.signal.throwIfAborted()
    report('importing', true)
    storeSample(database, sampler.kept(), () => Atomics.load(cancelled, 0) !== 0)
    report('done', true)
  } catch (cause) {
    const aborted = controller.signal.aborted || Atomics.load(cancelled, 0) !== 0
    parentPort!.postMessage({
      progress: {
        phase: aborted ? 'cancelled' : 'failed',
        received,
        total,
        kept: 0,
        message: aborted ? undefined : cause instanceof Error ? cause.message : String(cause),
      },
    })
    if (!aborted) throw cause
  } finally {
    parentPort!.postMessage({ usage: { requests: 0, bytes: received } })
    running = undefined
  }
  return readStatus(database, false)
}
parentPort!.on(
  'message',
  (message: {
    id: number
    method: string
    query?: LocalPuzzleQuery & LocalLadderQuery
    url?: string
    cancellation?: SharedArrayBuffer
  }) => {
    if (message.method === 'cancel') {
      running?.abort()
      return
    }
    void (async () => {
      try {
        let result: unknown
        if (message.method === 'status') result = readStatus(database, Boolean(running))
        else if (message.method === 'query') result = queryPuzzles(database, message.query!)
        else if (message.method === 'ladder') result = queryLadder(database, message.query!)
        else if (message.method === 'delete') {
          if (running) throw new Error('Cancel the download before deleting puzzles.')
          clearStored(database)
          result = readStatus(database, false)
        } else if (message.method === 'install')
          result = await install(message.url!, new Int32Array(message.cancellation!))
        else throw new Error('Unknown puzzle operation.')
        parentPort!.postMessage({ id: message.id, result })
      } catch (cause) {
        logWarn('puzzles', `Puzzle operation ${message.method} failed`, cause)
        parentPort!.postMessage({
          id: message.id,
          error: cause instanceof Error ? cause.message : String(cause),
          errorStack: cause instanceof Error ? (cause.stack ?? '').slice(0, 2000) : undefined,
        })
      }
    })()
  },
)
