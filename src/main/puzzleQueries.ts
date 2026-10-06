import type { DatabaseSync } from 'node:sqlite'
import { puzzleFromDb, type DbPuzzle } from '../shared/puzzle.ts'
import type { LocalLadderQuery, LocalPuzzleQuery, Puzzle, PuzzleDbStatus } from '../shared/types'

interface PuzzleRow {
  id: string
  fen: string
  moves: string
  rating: number
  plays: number
  themes: string
}

export function readStatus(database: DatabaseSync, busy: boolean): PuzzleDbStatus {
  const meta = database
    .prepare('SELECT importedAt, count, bytes FROM puzzle_meta WHERE id = 1')
    .get() as { importedAt: number; count: number; bytes: number } | undefined
  return {
    installed: Boolean(meta && meta.count > 0),
    count: meta?.count ?? 0,
    bytes: meta?.bytes ?? 0,
    importedAt: meta?.importedAt,
    busy,
  }
}

/** Replace the stored puzzles with `rows` in one transaction; a failure keeps the old ones. */
export function storeSample(
  database: DatabaseSync,
  rows: DbPuzzle[],
  cancelled: () => boolean = () => false,
): number {
  const insert = database.prepare(
    'INSERT OR IGNORE INTO puzzles (id, fen, moves, rating, plays, themes) VALUES (?, ?, ?, ?, ?, ?)',
  )
  database.exec('BEGIN IMMEDIATE')
  try {
    database.exec('DELETE FROM puzzles')
    let count = 0
    let bytes = 0
    for (const row of rows) {
      // Skip anything the app could not play (a malformed line).
      if (!puzzleFromDb(row)) continue
      if (cancelled()) throw new Error('Puzzle import cancelled.')
      const result = insert.run(row.id, row.fen, row.moves, row.rating, row.plays ?? 0, row.themes)
      count += Number(result.changes)
      if (result.changes) bytes += Buffer.byteLength(row.id + row.fen + row.moves + row.themes) + 24
    }
    if (!count) throw new Error('The downloaded file held no usable puzzles.')
    if (cancelled()) throw new Error('Puzzle import cancelled.')
    database
      .prepare(
        'INSERT OR REPLACE INTO puzzle_meta (id, importedAt, count, bytes) VALUES (1, ?, ?, ?)',
      )
      .run(Date.now(), count, bytes)
    database.exec('COMMIT')
    return count
  } catch (cause) {
    database.exec('ROLLBACK')
    throw cause
  }
}

export function clearStored(database: DatabaseSync): void {
  database.exec('DELETE FROM puzzles; DELETE FROM puzzle_meta;')
  // Give the space back to the file rather than leave it for reuse.
  database.exec('VACUUM')
}

function requireInstalled(database: DatabaseSync): void {
  if (!database.prepare('SELECT count FROM puzzle_meta WHERE id = 1 AND count > 0').get())
    throw new Error(
      'Download the puzzle database first (Settings → Data & storage, or the Rush tab).',
    )
}

const toPuzzles = (rows: PuzzleRow[]): Puzzle[] => rows.flatMap((row) => puzzleFromDb(row) ?? [])

/** Indexed candidate pool (no sort): the rating index answers the range, randomness happens in JS. */
const CANDIDATE_CAP = 5_000

function takeRandom<T>(items: T[], count: number): T[] {
  const pool = [...items]
  const keep = Math.max(0, Math.min(count, pool.length))
  for (let i = 0; i < keep; i++) {
    const j = i + Math.floor(Math.random() * (pool.length - i))
    ;[pool[i], pool[j]] = [pool[j]!, pool[i]!]
  }
  return pool.slice(0, keep)
}

const hasTheme = (row: PuzzleRow, theme: string): boolean =>
  ` ${row.themes} `.includes(` ${theme} `)

/** Random unused rows from one rating window with a single indexed seek per attempt (no sort). */
function pickWindow(database: DatabaseSync, min: number, max: number, used: Set<string>): Puzzle[] {
  const total = (
    database
      .prepare('SELECT COUNT(*) AS n FROM puzzles WHERE rating BETWEEN ? AND ?')
      .get(min, max) as unknown as { n: number }
  ).n
  if (!total) return []
  const offset = Math.floor(Math.random() * total)
  const rows = database
    .prepare(
      'SELECT id, fen, moves, rating, plays, themes FROM puzzles WHERE rating BETWEEN ? AND ? LIMIT 8 OFFSET ?',
    )
    .all(min, max, offset) as unknown as PuzzleRow[]
  return toPuzzles(rows).filter((puzzle) => !used.has(puzzle.id))
}

/** Random puzzles, optionally of one theme and rating range. */
export function queryPuzzles(database: DatabaseSync, query: LocalPuzzleQuery): Puzzle[] {
  requireInstalled(database)
  const min = query.minRating ?? 0
  const max = query.maxRating ?? 4000
  const count = Math.max(0, query.count)
  if (!count) return []
  const rows = database
    .prepare(
      `SELECT id, fen, moves, rating, plays, themes FROM puzzles WHERE rating BETWEEN ? AND ? LIMIT ?`,
    )
    .all(min, max, CANDIDATE_CAP) as unknown as PuzzleRow[]
  const pool =
    query.theme && query.theme !== 'mix' ? rows.filter((row) => hasTheme(row, query.theme!)) : rows
  const pickedRows = takeRandom(pool, count)
  // Rare theme in a wide range can truncate below `count`: top up with indexed random seeks.
  if (pickedRows.length < count && query.theme && query.theme !== 'mix') {
    const seen = new Set(pool.map((row) => row.id))
    const total = (
      database
        .prepare('SELECT COUNT(*) AS n FROM puzzles WHERE rating BETWEEN ? AND ?')
        .get(min, max) as unknown as { n: number }
    ).n
    for (let attempt = 0; attempt < 200 && pickedRows.length < count && total > 0; attempt++) {
      const offset = Math.floor(Math.random() * total)
      const row = (
        database
          .prepare(
            'SELECT id, fen, moves, rating, plays, themes FROM puzzles WHERE rating BETWEEN ? AND ? LIMIT 1 OFFSET ?',
          )
          .all(min, max, offset) as unknown as PuzzleRow[]
      )[0]
      if (row && !seen.has(row.id) && hasTheme(row, query.theme)) {
        seen.add(row.id)
        pickedRows.push(row)
      }
    }
  }
  return toPuzzles(pickedRows)
}

/** One random puzzle near each step from `from` up to `to`: an easy-to-hard run. */
export function queryLadder(database: DatabaseSync, query: LocalLadderQuery): Puzzle[] {
  requireInstalled(database)
  const used = new Set<string>()
  const ladder: Puzzle[] = []
  for (let step = 0; step < query.count; step++) {
    const target = Math.round(
      query.from + ((query.to - query.from) * step) / Math.max(1, query.count - 1),
    )
    // Widen the window until something unused turns up (the sample is thin at the extremes).
    for (const reach of [30, 80, 200, 500]) {
      const pick = pickWindow(database, target - reach, target + reach, used).find(
        (puzzle) => !used.has(puzzle.id),
      )
      if (pick) {
        used.add(pick.id)
        ladder.push(pick)
        break
      }
    }
  }
  return ladder
}
