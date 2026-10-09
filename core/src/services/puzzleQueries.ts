import type { DatabaseSync } from 'node:sqlite'
import { puzzleFromDb, type DbPuzzle } from '../domain/puzzle.ts'
import type { LocalLadderQuery, LocalPuzzleQuery, Puzzle, PuzzleDbStatus } from '../contracts/types'

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
  // Partial Fisher-Yates in place: `pool` is freshly fetched per query, so no
  // caller observes the reorder, and a 5000-row candidate pool avoids a copy.
  const keep = Math.max(0, Math.min(count, items.length))
  for (let i = 0; i < keep; i++) {
    const j = i + Math.floor(Math.random() * (items.length - i))
    ;[items[i], items[j]] = [items[j]!, items[i]!]
  }
  return items.slice(0, keep)
}

const hasTheme = (row: PuzzleRow, theme: string): boolean =>
  ` ${row.themes} `.includes(` ${theme} `)

/**
 * Random unused rows from one rating window with indexed seeks (no COUNT, no
 * OFFSET walk, no sort). A random pivot splits the window into two
 * index-range scans, so a 25k-row window costs two small seeks instead of
 * walking up to 25k index entries.
 */
function pickWindow(database: DatabaseSync, min: number, max: number, used: Set<string>): Puzzle[] {
  if (min > max) return []
  const pivot = min + Math.random() * (max - min)
  const columns = 'SELECT id, fen, moves, rating, plays, themes FROM puzzles'
  const rows = database
    .prepare(`${columns} WHERE rating BETWEEN ? AND ? AND rating >= ? LIMIT 8`)
    .all(min, max, pivot) as unknown as PuzzleRow[]
  const combined =
    rows.length < 8
      ? [
          ...rows,
          ...(
            database
              .prepare(`${columns} WHERE rating BETWEEN ? AND ? AND rating < ? LIMIT 8`)
              .all(min, max, pivot) as unknown as PuzzleRow[]
          ).filter((row) => !rows.some((other) => other.id === row.id)),
        ]
      : rows
  return toPuzzles(combined).filter((puzzle) => !used.has(puzzle.id))
}

/** Random puzzles, optionally of one theme and rating range. */
export function queryPuzzles(database: DatabaseSync, query: LocalPuzzleQuery): Puzzle[] {
  requireInstalled(database)
  const min = query.minRating ?? 0
  const max = query.maxRating ?? 4000
  const count = Math.max(0, query.count)
  if (!count) return []
  const theme = query.theme && query.theme !== 'mix' ? query.theme : undefined
  const columns = 'SELECT id, fen, moves, rating, plays, themes FROM puzzles'
  // The theme prefilter runs in SQLite so a rare theme no longer transfers 5000
  // non-matching rows; `hasTheme` below stays authoritative for exact tokens
  // (`_` is a LIKE wildcard, so SQL may over-match).
  const rows = (theme
    ? database
        .prepare(
          `${columns} WHERE rating BETWEEN ? AND ? AND (' ' || themes || ' ' LIKE ' %' || ? || ' %') LIMIT ?`,
        )
        .all(min, max, theme, CANDIDATE_CAP)
    : database
        .prepare(`${columns} WHERE rating BETWEEN ? AND ? LIMIT ?`)
        .all(min, max, CANDIDATE_CAP)) as unknown as PuzzleRow[]
  const pool = theme ? rows.filter((row) => hasTheme(row, theme)) : rows
  const pickedRows = takeRandom(pool, count)
  // Rare theme in a wide range can truncate below `count`: top up with indexed
  // pivot seeks (up to 8 candidates per seek, no OFFSET walk).
  if (pickedRows.length < count && theme) {
    const seen = new Set(pool.map((row) => row.id))
    const like = `(' ' || themes || ' ' LIKE ' %' || ? || ' %')`
    for (let attempt = 0; attempt < 25 && pickedRows.length < count; attempt++) {
      const pivot = min + Math.random() * Math.max(0, max - min)
      const candidates = [
        ...(
          database
            .prepare(`${columns} WHERE rating BETWEEN ? AND ? AND rating >= ? AND ${like} LIMIT 8`)
            .all(min, max, pivot, theme) as unknown as PuzzleRow[]
        ).filter((row) => !seen.has(row.id)),
      ]
      if (candidates.length < 8) {
        for (const row of database
          .prepare(`${columns} WHERE rating BETWEEN ? AND ? AND rating < ? AND ${like} LIMIT 8`)
          .all(min, max, pivot, theme) as unknown as PuzzleRow[]) {
          if (!seen.has(row.id)) candidates.push(row)
          if (candidates.length >= 8) break
        }
      }
      for (const row of candidates) {
        if (pickedRows.length >= count) break
        if (seen.has(row.id) || !hasTheme(row, theme)) continue
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
