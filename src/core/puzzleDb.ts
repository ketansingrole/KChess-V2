import { Worker } from 'node:worker_threads'
import { join } from 'node:path'
import { dbPath, getDb } from './db'
import { errorSummary, logError, logWarn } from './logger'
import { platform } from './platform'
import { recordUsage } from './usage'
import type {
  LocalLadderQuery,
  LocalPuzzleQuery,
  Puzzle,
  PuzzleDbProgress,
  PuzzleDbStatus,
} from '../shared/types'
export const PUZZLE_DB_URL = 'https://database.lichess.org/lichess_db_puzzle.csv.zst'
let worker: Worker | undefined
let nextId = 0
const waiting = new Map<
  number,
  { resolve: (value: unknown) => void; reject: (error: Error) => void }
>()
let progress: ((event: PuzzleDbProgress) => void) | undefined
let cancellation: Int32Array | undefined
function service(): Worker {
  if (worker) return worker
  getDb() // Ensure legacy schema/migrations are ready before the worker reads it.
  const self = new Worker(
    platform().puzzleWorkerPath ?? join(import.meta.dirname, 'puzzleWorker.js'),
    {
      workerData: { path: join(platform().dataDir, 'puzzles.db'), legacyPath: dbPath() },
    },
  )
  worker = self
  const failed = (error: Error): void => {
    if (worker !== self) return
    worker = undefined
    const pending = waiting.size
    logError('puzzles', 'Puzzle service failed:', `pending=${pending}`, errorSummary(error))
    for (const entry of waiting.values()) entry.reject(error)
    waiting.clear()
  }
  self.on(
    'message',
    (message: {
      id?: number
      error?: string
      errorStack?: string
      result?: unknown
      progress?: PuzzleDbProgress
      usage?: { requests: number; bytes: number }
    }) => {
      if (worker !== self) return
      if (message.progress) progress?.(message.progress)
      if (message.usage) recordUsage('', 'database', message.usage.requests, message.usage.bytes)
      if (message.id !== undefined) {
        const entry = waiting.get(message.id)
        waiting.delete(message.id)
        if (message.error) {
          const error = new Error(message.error)
          // Worker stacks name the worker frames; keep them for handover debugging.
          if (message.errorStack)
            error.stack = `${error.stack ?? ''}\nCaused by worker: ${message.errorStack.slice(0, 1000)}`
          logWarn('puzzles', 'Puzzle operation failed:', errorSummary(error))
          entry?.reject(error)
        } else entry?.resolve(message.result)
      }
    },
  )
  self.on('error', failed)
  self.on('exit', () => failed(new Error('Puzzle service stopped. Retry the operation.')))
  return self
}
function call<T>(method: string, options: object = {}): Promise<T> {
  const self = service()
  const id = ++nextId
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve: (value) => resolve(value as T), reject })
    self.postMessage({ id, method, ...options })
  })
}
export const puzzleDbStatus = (): Promise<PuzzleDbStatus> => call('status')
export function cancelPuzzleDb(): void {
  if (cancellation) Atomics.store(cancellation, 0, 1)
  worker?.postMessage({ method: 'cancel' })
}
export async function installPuzzleDb(
  onProgress: (event: PuzzleDbProgress) => void,
  url = PUZZLE_DB_URL,
): Promise<PuzzleDbStatus> {
  if (cancellation) throw new Error('The puzzle database is already downloading.')
  progress = onProgress
  cancellation = new Int32Array(new SharedArrayBuffer(4))
  try {
    return await call('install', { url, cancellation: cancellation.buffer })
  } finally {
    cancellation = undefined
    progress = undefined
  }
}
export const deletePuzzleDb = (): Promise<PuzzleDbStatus> => call('delete')
export const localPuzzles = (query: LocalPuzzleQuery): Promise<Puzzle[]> => call('query', { query })
export const localLadder = (query: LocalLadderQuery): Promise<Puzzle[]> => call('ladder', { query })
export function closePuzzleWorker(): void {
  cancelPuzzleDb()
  for (const entry of waiting.values()) entry.reject(new Error('Puzzle service closed.'))
  waiting.clear()
  void worker?.terminate()
  worker = undefined
}
