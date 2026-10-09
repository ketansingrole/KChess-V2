import type { Readable } from 'node:stream'
import { createZstdDecompress } from 'node:zlib'
import type { DbPuzzle } from '../domain/puzzle'

/**
 * Lichess publishes every rated puzzle as one CC0 file. KChess streams it once, keeps a random,
 * well-tested sample spread over every rating and theme (around a hundred thousand puzzles, a few
 * tens of megabytes) and forgets the rest, so Storm, Streak, Rush and offline practice have
 * puzzles of the right difficulty without keeping the multi-gigabyte original.
 */
const RATING_BUCKET = 50
const PER_BUCKET = 1800
const PER_THEME = 400

/** Puzzles the community rated well and that have been played enough for their rating to be trusted. */
const isSolid = (deviation: number, popularity: number, plays: number): boolean =>
  deviation <= 90 && popularity >= 85 && plays >= 300
const isSolidForTheme = (deviation: number, popularity: number, plays: number): boolean =>
  deviation <= 110 && popularity >= 70 && plays >= 100

/** Uniform random sample of a stream of unknown length. */
class Reservoir {
  private seen = 0
  readonly rows: DbPuzzle[] = []
  private readonly size: number
  constructor(size: number) {
    this.size = size
  }
  add(row: DbPuzzle): void {
    this.seen++
    if (this.rows.length < this.size) this.rows.push(row)
    else {
      const slot = Math.floor(Math.random() * this.seen)
      if (slot < this.size) this.rows[slot] = row
    }
  }
}

/** Line parser and sampler for the CSV: `PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes,…`. */
export class PuzzleSampler {
  private readonly buckets = new Map<number, Reservoir>()
  private readonly themes = new Map<string, Reservoir>()
  lines = 0

  add(line: string): void {
    if (line.length > 16_000) throw new Error('Puzzle database contains an oversized CSV line.')
    const cells = line.split(',')
    if (cells.length < 8 || cells[0] === 'PuzzleId') return
    const rating = Number(cells[3])
    const deviation = Number(cells[4])
    const popularity = Number(cells[5])
    const plays = Number(cells[6])
    if (!Number.isFinite(rating) || rating < 0 || rating > 4000 || !Number.isFinite(deviation))
      return
    if (!/^[a-zA-Z0-9]+$/.test(cells[0]!) || cells[0]!.length > 64) return
    if (cells[7]!.length > 256 || !/^[a-zA-Z0-9 ]*$/.test(cells[7]!)) return
    this.lines++
    const solid = isSolid(deviation, popularity, plays)
    const solidForTheme = isSolidForTheme(deviation, popularity, plays)
    if (!solid && !solidForTheme) return
    const row: DbPuzzle = {
      id: cells[0]!,
      fen: cells[1]!,
      moves: cells[2]!,
      rating,
      plays,
      themes: cells[7]!,
    }
    if (solid) {
      const bucket = Math.floor(rating / RATING_BUCKET)
      let reservoir = this.buckets.get(bucket)
      if (!reservoir) this.buckets.set(bucket, (reservoir = new Reservoir(PER_BUCKET)))
      reservoir.add(row)
    }
    if (solidForTheme)
      for (const theme of row.themes.split(' ')) {
        if (!theme || theme.length > 40) continue
        let reservoir = this.themes.get(theme)
        if (!reservoir) {
          if (this.themes.size >= 256) continue
          this.themes.set(theme, (reservoir = new Reservoir(PER_THEME)))
        }
        reservoir.add(row)
      }
  }

  /** Cheap bounded progress counter; exact deduplication is only done at import. */
  get count(): number {
    return [...this.buckets.values()].reduce((sum, bucket) => sum + bucket.rows.length, 0)
  }

  /** Every puzzle kept, once. */
  kept(): DbPuzzle[] {
    const unique = new Map<string, DbPuzzle>()
    for (const reservoir of [...this.buckets.values(), ...this.themes.values()])
      for (const row of reservoir.rows) unique.set(row.id, row)
    return [...unique.values()]
  }
}

/** Feed a zstd-compressed CSV to `sampler` as it arrives, never holding more than a chunk of it. */
export async function sampleZstdCsv(
  source: Readable,
  sampler: PuzzleSampler,
  signal: AbortSignal,
): Promise<void> {
  const decompressed = source.pipe(createZstdDecompress())
  source.on('error', (cause) => decompressed.destroy(cause))
  const decoder = new TextDecoder()
  let tail = ''
  for await (const chunk of decompressed as AsyncIterable<Buffer>) {
    if (signal.aborted) throw new DOMException('Cancelled', 'AbortError')
    const lines = (tail + decoder.decode(chunk, { stream: true })).split('\n')
    tail = lines.pop() ?? ''
    if (tail.length > 16_000) throw new Error('Puzzle database contains an oversized CSV line.')
    for (const line of lines) sampler.add(line)
  }
  sampler.add(tail + decoder.decode())
}
