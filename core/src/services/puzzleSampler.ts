import type { Readable } from 'node:stream'
import { createZstdDecompress } from 'node:zlib'
import type { DbPuzzle } from '../domain/puzzle'

/**
 * Lichess publishes every rated puzzle as one CC0 file. KChess streams it once, keeps a random,
 * well-tested sample spread over every rating and theme (around a hundred thousand puzzles, a few
 * tens of megabytes) and forgets the rest, so Storm, Streak, Rush and offline practice have
 * puzzles of the right difficulty without keeping the multi-gigabyte original. The sampler is
 * native (`crates/kchess-domain/src/puzzle.rs`) and is fed the decompressed CSV as it arrives.
 */
export interface ChunkSampler {
  push(chunk: Uint8Array): void
  /** The last line, which has no newline after it. */
  finish(): void
  readonly count: number
  readonly lines: number
  kept(): DbPuzzle[]
}

/** Feed a zstd-compressed CSV to `sampler` as it arrives, never holding more than a chunk of it. */
export async function sampleZstdCsv(
  source: Readable,
  sampler: ChunkSampler,
  signal: AbortSignal,
): Promise<void> {
  const decompressed = source.pipe(createZstdDecompress())
  source.on('error', (cause) => decompressed.destroy(cause))
  for await (const chunk of decompressed as AsyncIterable<Buffer>) {
    if (signal.aborted) throw new DOMException('Cancelled', 'AbortError')
    sampler.push(chunk)
  }
  sampler.finish()
}
