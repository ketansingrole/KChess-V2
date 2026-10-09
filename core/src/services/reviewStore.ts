import { getDb } from './db'
import { logDebug } from './logger'
import { summarize } from '../domain/review'
import type { ReviewSummary, StoredReview } from '../contracts/types'

/**
 * Game reviews on disk. A review is the scores of every position; the summary beside it (accuracy
 * and counts per side) is what the game list shows without working anything out.
 */

interface ReviewRow {
  key: string
  data: string
  summary: string
}

function parse<T>(text: string): T | null {
  try {
    return JSON.parse(text) as T
  } catch (cause) {
    logDebug('review-store', 'Review parse failed:', cause)
    return null
  }
}

export function readReview(key: string): StoredReview | null {
  return readStored(key)?.review ?? null
}

function readStored(key: string): { review: StoredReview; summary: ReviewSummary | null } | null {
  const row = getDb()
    .prepare('SELECT key, data, summary FROM reviews WHERE key = ?')
    .get(key) as unknown as ReviewRow | undefined
  if (!row) return null
  const review = parse<StoredReview>(row.data)
  if (!review) return null
  return { review, summary: parse<ReviewSummary>(row.summary) }
}

/** Which review to keep: Lichess's own beats ours, a finished one beats one in progress. */
function rank(review: Pick<StoredReview, 'source' | 'complete'>): number {
  return (review.complete ? 2 : 0) + (review.source === 'lichess' ? 1 : 0)
}

/** Save `review` unless a better one is already stored; returns what is stored afterwards. */
export function writeReview(review: StoredReview): {
  review: StoredReview
  summary: ReviewSummary
} {
  const database = getDb()
  const link = (): void => {
    if (review.gameId)
      database
        .prepare('INSERT OR IGNORE INTO game_reviews (gameId, reviewKey) VALUES (?, ?)')
        .run(review.gameId, review.key)
  }
  const existing = readStored(review.key)
  if (existing && rank(existing.review) > rank(review)) {
    link()
    return {
      review: { ...existing.review, gameId: review.gameId ?? existing.review.gameId },
      summary: existing.summary ?? summarize(existing.review),
    }
  }
  const merged: StoredReview = {
    ...review,
    // A local review of a game opened from the list keeps the game it belongs to.
    gameId: review.gameId ?? existing?.review.gameId,
    updatedAt: Date.now(),
  }
  const summary = summarize(merged)
  getDb()
    .prepare(
      `INSERT INTO reviews (key, gameId, source, complete, depth, data, summary, updatedAt)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(key) DO UPDATE SET gameId=excluded.gameId, source=excluded.source,
         complete=excluded.complete, depth=excluded.depth, data=excluded.data,
         summary=excluded.summary, updatedAt=excluded.updatedAt`,
    )
    .run(
      merged.key,
      merged.gameId ?? null,
      merged.source,
      merged.complete ? 1 : 0,
      merged.depth,
      JSON.stringify(merged),
      JSON.stringify(summary),
      merged.updatedAt,
    )
  link()
  return { review: merged, summary }
}

/**
 * Summaries of these Lichess games' reviews, by game id; games without one are left out.
 * When a game has several, the finished, most recent one wins.
 */
export function reviewSummaries(ids: readonly string[]): Record<string, ReviewSummary> {
  if (!ids.length) return {}
  const rows = getDb()
    .prepare(
      `SELECT gr.gameId, r.summary FROM game_reviews gr JOIN reviews r ON r.key=gr.reviewKey WHERE gr.gameId IN (${ids.map(() => '?').join(', ')})
       ORDER BY r.complete, r.updatedAt`,
    )
    .all(...ids) as unknown as { gameId: string; summary: string }[]
  const result: Record<string, ReviewSummary> = {}
  for (const row of rows) {
    const summary = parse<ReviewSummary>(row.summary)
    if (summary) result[row.gameId] = summary
  }
  return result
}

/** Whether this account is still on this device (it may have been logged out or removed). */
export function hasAccount(username: string): boolean {
  return Boolean(
    getDb().prepare('SELECT 1 FROM accounts WHERE username = ? COLLATE NOCASE').get(username),
  )
}

/** Remember that Lichess was asked for these games' analysis. */
export function markChecked(ids: readonly string[], at = Date.now()): void {
  if (!ids.length) return
  const database = getDb()
  const insert = database.prepare(
    'INSERT OR REPLACE INTO lichess_review_checks (id, checkedAt) VALUES (?, ?)',
  )
  database.exec('BEGIN IMMEDIATE')
  try {
    for (const id of ids) insert.run(id, at)
    database.exec('COMMIT')
  } catch (cause) {
    database.exec('ROLLBACK')
    throw cause
  }
}

export interface GameToReview {
  id: string
  account: string
  moves: string
  pgn: string | null
  perf: string
  createdAt: number
  /** Lichess has already been asked for its analysis. */
  checked: boolean
}

/**
 * The accounts' finished games with no finished review, newest first. `since` limits them to games played
 * after it (ms).
 */
export function gamesToReview(accounts: readonly string[], since = 0, limit = 300): GameToReview[] {
  if (!accounts.length) return []
  const marks = accounts.map(() => '?').join(', ')
  const rows = getDb()
    .prepare(
      `SELECT g.id, g.account, g.moves, g.pgn, g.perf, g.createdAt, c.id IS NOT NULL AS checked
       FROM games g
       LEFT JOIN lichess_review_checks c ON c.id = g.id
       WHERE g.account COLLATE NOCASE IN (${marks}) AND g.createdAt >= ? AND NOT EXISTS (
         SELECT 1 FROM game_reviews gr JOIN reviews r ON r.key=gr.reviewKey
         WHERE gr.gameId=g.id AND r.complete=1)
         AND g.moves != '' AND g.status NOT IN ('created', 'started')
       GROUP BY g.id
       ORDER BY g.createdAt DESC
       LIMIT ?`,
    )
    .all(...accounts, since, limit) as unknown as (Omit<GameToReview, 'checked'> & {
    checked: number
  })[]
  return rows.map((row) => ({ ...row, checked: row.checked === 1 }))
}

/** Reviews kept, for the storage report. */
export function reviewCount(): number {
  return (getDb().prepare('SELECT COUNT(*) AS n FROM reviews').get() as unknown as { n: number }).n
}
