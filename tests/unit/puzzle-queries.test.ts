import { describe, expect, it } from 'vitest'
import { DatabaseSync } from 'node:sqlite'
import { queryLadder, queryPuzzles, storeSample } from '../../src/main/puzzleQueries'
import type { DbPuzzle } from '../../src/shared/puzzle'

const FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'

function row(id: string, rating: number, themes = 'mate mateIn1'): DbPuzzle {
  return { id, fen: FEN, moves: 'e2e4 e7e5 g1f3', rating, plays: 10, themes }
}

function setup(count = 500): DatabaseSync {
  const db = new DatabaseSync(':memory:')
  db.exec(
    'CREATE TABLE puzzles (id TEXT PRIMARY KEY, fen TEXT NOT NULL, moves TEXT NOT NULL, rating INTEGER NOT NULL, plays INTEGER NOT NULL DEFAULT 0, themes TEXT NOT NULL); CREATE INDEX idx_puzzles_rating ON puzzles(rating); CREATE TABLE puzzle_meta (id INTEGER PRIMARY KEY CHECK(id = 1), importedAt INTEGER NOT NULL, count INTEGER NOT NULL, bytes INTEGER NOT NULL DEFAULT 0);',
  )
  const themes = ['mate', 'fork', 'pin', 'mate mateIn2', 'fork pin']
  const rows: DbPuzzle[] = Array.from({ length: count }, (_, i) => ({
    ...row(`p${String(i).padStart(5, '0')}`, 800 + ((i * 37) % 1600)),
    themes: themes[i % themes.length]!,
  }))
  storeSample(db, rows)
  return db
}

describe('indexed puzzle sampling (no ORDER BY RANDOM)', () => {
  it('returns the requested count within the rating range', () => {
    const db = setup()
    try {
      const puzzles = queryPuzzles(db, { minRating: 1000, maxRating: 1500, count: 20 })
      expect(puzzles).toHaveLength(20)
      for (const puzzle of puzzles) {
        expect(puzzle.rating).toBeGreaterThanOrEqual(1000)
        expect(puzzle.rating).toBeLessThanOrEqual(1500)
      }
      expect(new Set(puzzles.map((p) => p.id)).size).toBe(20)
    } finally {
      db.close()
    }
  })

  it('matches whole theme tokens, not substrings', () => {
    const db = setup()
    try {
      const puzzles = queryPuzzles(db, {
        theme: 'mate',
        minRating: 0,
        maxRating: 4000,
        count: 30,
      })
      expect(puzzles.length).toBeGreaterThan(0)
      for (const puzzle of puzzles) expect(puzzle.themes).toContain('mate')
      // 'mateIn2' must not match a query for 'mateIn' (substring guard).
      const none = queryPuzzles(db, {
        theme: 'mateIn',
        minRating: 0,
        maxRating: 4000,
        count: 10,
      })
      expect(none).toHaveLength(0)
    } finally {
      db.close()
    }
  })

  it('uses the rating index (no temp b-tree sort)', () => {
    const db = setup()
    try {
      const plan = (
        db
          .prepare(
            'EXPLAIN QUERY PLAN SELECT id, fen, moves, rating, plays, themes FROM puzzles WHERE rating BETWEEN ? AND ? LIMIT ?',
          )
          .all(1000, 1500, 5000) as unknown as { detail: string }[]
      )
        .map((row) => row.detail)
        .join(' | ')
      expect(plan).toContain('idx_puzzles_rating')
      expect(plan).not.toContain('TEMP B-TREE')
    } finally {
      db.close()
    }
  })

  it('builds a ladder with increasing difficulty and no repeats', () => {
    const db = setup()
    try {
      const ladder = queryLadder(db, { from: 900, to: 2000, count: 6 })
      expect(ladder).toHaveLength(6)
      expect(new Set(ladder.map((p) => p.id)).size).toBe(6)
      const ratings = ladder.map((p) => p.rating)
      expect(ratings[0]).toBeLessThan(ratings[ratings.length - 1]!)
    } finally {
      db.close()
    }
  })
})
