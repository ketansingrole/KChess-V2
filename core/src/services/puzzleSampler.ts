import type { DbPuzzle } from '../domain/puzzle'

/**
 * Lichess publishes every rated puzzle as one CC0 file. KChess streams it once, keeps a random,
 * well-tested sample spread over every rating and theme (around a hundred thousand puzzles, a few
 * tens of megabytes) and forgets the rest, so Storm, Streak, Rush and offline practice have
 * puzzles of the right difficulty without keeping the multi-gigabyte original. The Rust core
 * downloads, decompresses and samples the file (`crates/kchess-core/src/puzzles`); the sampler is
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
