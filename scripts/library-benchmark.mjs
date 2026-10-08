import { mkdirSync, writeFileSync } from 'node:fs'
import { PERFORMANCE_BUDGETS, assertLibraryBudget } from './performance-budgets.mjs'
import { DatabaseSync } from 'node:sqlite'
import { migrate } from '../src/core/migrations.ts'

// Structural probe of list-row SQL/clone cost, not a renderer or full loadData benchmark.
const db = new DatabaseSync(':memory:')
migrate(db)
const insert = db.prepare(`INSERT INTO games
  (account, id, createdAt, lastMoveAt, rated, speed, perf, status, color, opponent, moves)
  VALUES (?, ?, ?, ?, 1, 'blitz', 'blitz', 'mate', 'white', 'rival', 'e4 e5 Nf3 Nc6 Bb5 a6')`)
const query =
  'SELECT account, id, createdAt, lastMoveAt, rated, speed, perf, status, winner, color, opponent, opponentRating, playerRating, ratingDiff, opening, moves FROM games ORDER BY createdAt DESC'
const samples = []
let installed = 0
for (const count of [1000, 5000, 50_000]) {
  db.exec('BEGIN')
  for (; installed < count; installed++) {
    const date = Date.UTC(2020, 0, 1) + installed * 60_000
    insert.run(
      `account${Math.floor(installed / 5000)}`,
      `G${String(installed).padStart(7, '0')}`,
      date,
      date,
    )
  }
  db.exec('COMMIT')
  const start = performance.now()
  const rows = db.prepare(query).all()
  const queried = performance.now()
  structuredClone(rows)
  const cloned = performance.now()
  const pageStart = performance.now()
  const page = db.prepare(query + ' LIMIT ?').all(PERFORMANCE_BUDGETS.libraryPageRows)
  const pageQueryMs = +(performance.now() - pageStart).toFixed(2)
  const sample = {
    count,
    queryMs: +(queried - start).toFixed(2),
    cloneMs: +(cloned - queried).toFixed(2),
    serializedBytes: Buffer.byteLength(JSON.stringify(rows)),
    pageRows: page.length,
    pageSerializedBytes: Buffer.byteLength(JSON.stringify(page)),
    pageQueryMs,
  }
  if (process.argv.includes('--check')) assertLibraryBudget(sample)
  samples.push(sample)
  console.info(JSON.stringify(sample))
}
db.close()

mkdirSync('test-results/guardrails', { recursive: true })
writeFileSync(
  'test-results/guardrails/library.json',
  JSON.stringify({ budgets: PERFORMANCE_BUDGETS, samples }, null, 2) + '\n',
)
