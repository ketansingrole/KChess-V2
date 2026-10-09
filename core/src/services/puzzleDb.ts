import { getDb } from './db'
import { errorSummary, logWarn } from './logger'
import { closeNativeCore, nativeCall, onNativeEvent } from './nativeCore'
import type {
  LocalLadderQuery,
  LocalPuzzleQuery,
  Puzzle,
  PuzzleDbProgress,
  PuzzleDbStatus,
} from '../contracts/types'
export const PUZZLE_DB_URL = 'https://database.lichess.org/lichess_db_puzzle.csv.zst'

/** `puzzles.db` belongs to the Rust core's puzzle service (`crates/kchess-core/src/puzzles`). */
async function call<T>(method: string, ...args: unknown[]): Promise<T> {
  getDb() // Ensure legacy schema/migrations are ready before the service reads it.
  try {
    return await nativeCall<T>(`puzzles.${method}`, ...args)
  } catch (cause) {
    logWarn('puzzles', 'Puzzle operation failed:', errorSummary(cause))
    throw cause
  }
}

export const puzzleDbStatus = (): Promise<PuzzleDbStatus> => call('status')
export function cancelPuzzleDb(): void {
  void call('cancel').catch((cause: unknown) => {
    logWarn('puzzles', 'Puzzle download cancel failed:', errorSummary(cause))
  })
}
export async function installPuzzleDb(
  onProgress: (event: PuzzleDbProgress) => void,
  url = PUZZLE_DB_URL,
): Promise<PuzzleDbStatus> {
  const stop = onNativeEvent<PuzzleDbProgress>('puzzles:progress', onProgress)
  try {
    return await call('install', url)
  } finally {
    stop()
  }
}
export const deletePuzzleDb = (): Promise<PuzzleDbStatus> => call('delete')
export const localPuzzles = (query: LocalPuzzleQuery): Promise<Puzzle[]> => call('query', query)
export const localLadder = (query: LocalLadderQuery): Promise<Puzzle[]> => call('ladder', query)
/** Stop the puzzle service with the rest of the Rust core. */
export const closePuzzleDb = (): Promise<void> => closeNativeCore()
