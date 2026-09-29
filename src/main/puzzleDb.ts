import { Readable } from 'node:stream'
import { getDb } from './db'
import { meteredFetch, withUsage } from './usage'
import { PuzzleSampler, sampleZstdCsv } from './puzzleSampler'
import { clearStored, queryLadder, queryPuzzles, readStatus, storeSample } from './puzzleQueries'
import type {
  LocalLadderQuery,
  LocalPuzzleQuery,
  Puzzle,
  PuzzleDbProgress,
  PuzzleDbStatus,
} from '../shared/types'

/** Lichess's public puzzle database (CC0); `puzzleSampler.ts` says what is kept of it. */
export const PUZZLE_DB_URL = 'https://database.lichess.org/lichess_db_puzzle.csv.zst'

let running: AbortController | null = null

export const puzzleDbStatus = (): PuzzleDbStatus => readStatus(getDb(), running !== null)

export function cancelPuzzleDb(): void {
  running?.abort()
}

/** Download, sample and store the puzzle database. Replaces the old copy only once the new one is complete. */
export async function installPuzzleDb(
  onProgress: (progress: PuzzleDbProgress) => void,
  url = PUZZLE_DB_URL,
): Promise<PuzzleDbStatus> {
  if (running) throw new Error('The puzzle database is already downloading.')
  const controller = new AbortController()
  running = controller
  const sampler = new PuzzleSampler()
  let received = 0
  let total: number | undefined
  let lastReport = 0
  const report = (phase: PuzzleDbProgress['phase'], message?: string, force = false): void => {
    const now = Date.now()
    if (!force && now - lastReport < 250) return
    lastReport = now
    onProgress({ phase, received, total, kept: sampler.kept().length, message })
  }
  try {
    const response = await withUsage('', 'database', () =>
      meteredFetch(url, { signal: controller.signal }),
    )
    if (!response.ok || !response.body)
      throw new Error(`Lichess answered ${response.status} for the puzzle database.`)
    total = Number(response.headers.get('content-length')) || undefined
    const source = Readable.fromWeb(response.body as never)
    source.on('data', (chunk: Buffer) => {
      received += chunk.byteLength
      report('downloading')
    })
    await sampleZstdCsv(source, sampler, controller.signal)
    report('importing', undefined, true)
    const count = storeSample(getDb(), sampler.kept())
    if (!count) throw new Error('The downloaded file held no usable puzzles.')
    report('done', undefined, true)
  } catch (cause) {
    if (controller.signal.aborted) onProgress({ phase: 'cancelled', received, total, kept: 0 })
    else {
      const message = cause instanceof Error ? cause.message : String(cause)
      onProgress({ phase: 'failed', received, total, kept: 0, message })
      throw cause
    }
  } finally {
    running = null
  }
  return puzzleDbStatus()
}

export function deletePuzzleDb(): PuzzleDbStatus {
  clearStored(getDb())
  return puzzleDbStatus()
}

export const localPuzzles = (query: LocalPuzzleQuery): Puzzle[] => queryPuzzles(getDb(), query)
export const localLadder = (query: LocalLadderQuery): Puzzle[] => queryLadder(getDb(), query)
