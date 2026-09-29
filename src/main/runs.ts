import { getDb } from './db'
import type { RunInput, RunKind, RunRecord, RunSaved, RunSummary } from '../shared/types'

interface RunRow {
  id: number
  kind: string
  variant: string
  score: number
  detail: string
  playedAt: number
}

const RECENT = 20

function toRecord(row: RunRow): RunRecord {
  let detail: RunRecord['detail'] = {}
  try {
    detail = JSON.parse(row.detail) as RunRecord['detail']
  } catch {
    // A damaged row still shows its score.
  }
  return {
    id: row.id,
    kind: row.kind as RunKind,
    variant: row.variant,
    score: row.score,
    detail,
    playedAt: row.playedAt,
  }
}

export function runSummary(kind: RunKind): RunSummary {
  const database = getDb()
  const best = database
    .prepare(
      `SELECT variant, MAX(score) AS score,
        (SELECT r2.playedAt FROM runs r2 WHERE r2.kind = r.kind AND r2.variant = r.variant
          ORDER BY r2.score DESC, r2.playedAt ASC LIMIT 1) AS playedAt
       FROM runs r WHERE kind = ? GROUP BY variant`,
    )
    .all(kind) as unknown as { variant: string; score: number; playedAt: number }[]
  const recent = database
    .prepare('SELECT * FROM runs WHERE kind = ? ORDER BY playedAt DESC, id DESC LIMIT ?')
    .all(kind, RECENT) as unknown as RunRow[]
  const total = (
    database.prepare('SELECT COUNT(*) AS n FROM runs WHERE kind = ?').get(kind) as unknown as {
      n: number
    }
  ).n
  return {
    kind,
    best: Object.fromEntries(
      best.map((row) => [row.variant, { score: row.score, playedAt: row.playedAt }]),
    ),
    recent: recent.map(toRecord),
    total,
  }
}

export function saveRun(run: RunInput): RunSaved {
  const database = getDb()
  const previous = database
    .prepare('SELECT MAX(score) AS score FROM runs WHERE kind = ? AND variant = ?')
    .get(run.kind, run.variant) as unknown as { score: number | null }
  database
    .prepare('INSERT INTO runs (kind, variant, score, detail, playedAt) VALUES (?, ?, ?, ?, ?)')
    .run(run.kind, run.variant, run.score, JSON.stringify(run.detail), Date.now())
  return {
    summary: runSummary(run.kind),
    isBest: run.score > 0 && (previous.score === null || run.score > previous.score),
  }
}

/** Forget the local scores of one kind, or of all of them. */
export function clearRuns(kind?: RunKind): void {
  const database = getDb()
  if (kind) database.prepare('DELETE FROM runs WHERE kind = ?').run(kind)
  else database.exec('DELETE FROM runs')
}
