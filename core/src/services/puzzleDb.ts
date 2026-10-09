import { scopedState, bindCoreCallback, platform } from './platform'
import { Worker } from 'node:worker_threads'
import { join } from 'node:path'
import { dbPath, getDb } from './db'
import { errorSummary, logError, logWarn } from './logger'
import { recordUsage } from './usage'
import type {
  LocalLadderQuery,
  LocalPuzzleQuery,
  Puzzle,
  PuzzleDbProgress,
  PuzzleDbStatus,
} from '../contracts/types'
export const PUZZLE_DB_URL = 'https://database.lichess.org/lichess_db_puzzle.csv.zst'

function service(): Worker {
  if (serviceState.worker) return serviceState.worker
  getDb() // Ensure legacy schema/migrations are ready before the worker reads it.
  const self = new Worker(
    platform().puzzleWorkerPath ?? join(import.meta.dirname, 'puzzleWorker.js'),
    {
      workerData: { path: join(platform().dataDir, 'puzzles.db'), legacyPath: dbPath() },
    },
  )
  serviceState.worker = self
  const failed = (error: Error): void => {
    if (serviceState.worker !== self) return
    serviceState.worker = undefined
    const pending = serviceState.waiting.size
    logError('puzzles', 'Puzzle service failed:', `pending=${pending}`, errorSummary(error))
    for (const entry of serviceState.waiting.values()) entry.reject(error)
    serviceState.waiting.clear()
  }
  self.on(
    'message',
    bindCoreCallback(
      (message: {
        id?: number
        error?: string
        errorStack?: string
        result?: unknown
        progress?: PuzzleDbProgress
        usage?: { requests: number; bytes: number }
      }) => {
        if (serviceState.worker !== self) return
        if (message.progress) serviceState.progress?.(message.progress)
        if (message.usage) recordUsage('', 'database', message.usage.requests, message.usage.bytes)
        if (message.id !== undefined) {
          const entry = serviceState.waiting.get(message.id)
          serviceState.waiting.delete(message.id)
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
    ),
  )
  self.on('error', bindCoreCallback(failed))
  self.on(
    'exit',
    bindCoreCallback(() => failed(new Error('Puzzle service stopped. Retry the operation.'))),
  )
  return self
}
function call<T>(method: string, options: object = {}): Promise<T> {
  const self = service()
  const id = ++serviceState.nextId
  return new Promise((resolve, reject) => {
    serviceState.waiting.set(id, { resolve: (value) => resolve(value as T), reject })
    self.postMessage({ id, method, ...options })
  })
}
export const puzzleDbStatus = (): Promise<PuzzleDbStatus> => call('status')
export function cancelPuzzleDb(): void {
  if (serviceState.cancellation) Atomics.store(serviceState.cancellation, 0, 1)
  serviceState.worker?.postMessage({ method: 'cancel' })
}
export async function installPuzzleDb(
  onProgress: (event: PuzzleDbProgress) => void,
  url = PUZZLE_DB_URL,
): Promise<PuzzleDbStatus> {
  if (serviceState.cancellation) throw new Error('The puzzle database is already downloading.')
  serviceState.progress = onProgress
  serviceState.cancellation = new Int32Array(new SharedArrayBuffer(4))
  try {
    return await call('install', { url, cancellation: serviceState.cancellation.buffer })
  } finally {
    serviceState.cancellation = undefined
    serviceState.progress = undefined
  }
}
export const deletePuzzleDb = (): Promise<PuzzleDbStatus> => call('delete')
export const localPuzzles = (query: LocalPuzzleQuery): Promise<Puzzle[]> => call('query', { query })
export const localLadder = (query: LocalLadderQuery): Promise<Puzzle[]> => call('ladder', { query })
export async function closePuzzleWorker(): Promise<void> {
  cancelPuzzleDb()
  for (const entry of serviceState.waiting.values())
    entry.reject(new Error('Puzzle service closed.'))
  serviceState.waiting.clear()
  const previous = serviceState.worker
  serviceState.worker = undefined
  if (previous) await previous.terminate()
  serviceState.cancellation = undefined
  serviceState.progress = undefined
}

const serviceState = scopedState(() => ({
  worker: undefined as Worker | undefined,
  nextId: 0,
  waiting: new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>(),
  progress: undefined as ((event: PuzzleDbProgress) => void) | undefined,
  cancellation: undefined as Int32Array | undefined,
}))
