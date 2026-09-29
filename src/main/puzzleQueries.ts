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
  const meta = database.prepare('SELECT importedAt, count FROM puzzle_meta WHERE id = 1').get() as
    { importedAt: number; count: number } | undefined
  const size = database
    .prepare(
      'SELECT SUM(LENGTH(id) + LENGTH(fen) + LENGTH(moves) + LENGTH(themes) + 24) AS bytes FROM puzzles',
    )
    .get() as unknown as { bytes: number | null }
  return {
    installed: Boolean(meta && meta.count > 0),
    count: meta?.count ?? 0,
    bytes: size.bytes ?? 0,
    importedAt: meta?.importedAt,
    busy,
  }
}

/** Replace the stored puzzles with `rows` in one transaction; a failure keeps the old ones. */
export function storeSample(database: DatabaseSync, rows: DbPuzzle[]): number {
  const insert = database.prepare(
    'INSERT OR IGNORE INTO puzzles (id, fen, moves, rating, plays, themes) VALUES (?, ?, ?, ?, ?, ?)',
  )
  database.exec('BEGIN IMMEDIATE')
  try {
    database.exec('DELETE FROM puzzles')
    let count = 0
    for (const row of rows) {
      // Skip anything the app could not play (a malformed line).
      if (!puzzleFromDb(row)) continue
      insert.run(row.id, row.fen, row.moves, row.rating, row.plays ?? 0, row.themes)
      count++
    }
    database
      .prepare('INSERT OR REPLACE INTO puzzle_meta (id, importedAt, count) VALUES (1, ?, ?)')
      .run(Date.now(), count)
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
  if (!readStatus(database, false).installed)
    throw new Error(
      'Download the puzzle database first (Settings → Data & storage, or the Rush tab).',
    )
}

const toPuzzles = (rows: PuzzleRow[]): Puzzle[] => rows.flatMap((row) => puzzleFromDb(row) ?? [])

/** Random puzzles, optionally of one theme and rating range. */
export function queryPuzzles(database: DatabaseSync, query: LocalPuzzleQuery): Puzzle[] {
  requireInstalled(database)
  const clauses = ['rating BETWEEN ? AND ?']
  const params: (string | number)[] = [query.minRating ?? 0, query.maxRating ?? 4000]
  if (query.theme && query.theme !== 'mix') {
    clauses.push("(' ' || themes || ' ') LIKE ?")
    params.push(`% ${query.theme} %`)
  }
  const rows = database
    .prepare(
      `SELECT id, fen, moves, rating, plays, themes FROM puzzles WHERE ${clauses.join(' AND ')} ORDER BY RANDOM() LIMIT ?`,
    )
    .all(...params, query.count) as unknown as PuzzleRow[]
  return toPuzzles(rows)
}

/** One random puzzle near each step from `from` up to `to`: an easy-to-hard run. */
export function queryLadder(database: DatabaseSync, query: LocalLadderQuery): Puzzle[] {
  requireInstalled(database)
  const find = database.prepare(
    'SELECT id, fen, moves, rating, plays, themes FROM puzzles WHERE rating BETWEEN ? AND ? ORDER BY RANDOM() LIMIT 8',
  )
  const used = new Set<string>()
  const ladder: Puzzle[] = []
  for (let step = 0; step < query.count; step++) {
    const target = Math.round(
      query.from + ((query.to - query.from) * step) / Math.max(1, query.count - 1),
    )
    // Widen the window until something unused turns up (the sample is thin at the extremes).
    for (const reach of [30, 80, 200, 500]) {
      const rows = find.all(target - reach, target + reach) as unknown as PuzzleRow[]
      const pick = toPuzzles(rows).find((puzzle) => !used.has(puzzle.id))
      if (pick) {
        used.add(pick.id)
        ladder.push(pick)
        break
      }
    }
  }
  return ladder
}
