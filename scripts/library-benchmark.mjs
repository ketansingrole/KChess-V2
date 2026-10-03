import { DatabaseSync } from 'node:sqlite'
import { migrate } from '../src/main/migrations.ts'

// Structural probe of list-row SQL/clone cost, not a renderer or full loadData benchmark.
const db = new DatabaseSync(':memory:')
migrate(db)
const insert = db.prepare(`INSERT INTO games
  (account, id, createdAt, lastMoveAt, rated, speed, perf, status, color, opponent, moves)
  VALUES (?, ?, ?, ?, 1, 'blitz', 'blitz', 'mate', 'white', 'rival', 'e4 e5 Nf3 Nc6 Bb5 a6')`)
const query =
  'SELECT account, id, createdAt, lastMoveAt, rated, speed, perf, status, winner, color, opponent, opponentRating, playerRating, ratingDiff, opening, moves FROM games ORDER BY createdAt DESC'
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
  const page = db.prepare(query + ' LIMIT 100').all()
  const pageQueryMs = +(performance.now() - pageStart).toFixed(2)
  console.info(
    JSON.stringify({
      count,
      queryMs: +(queried - start).toFixed(2),
      cloneMs: +(cloned - queried).toFixed(2),
      serializedBytes: Buffer.byteLength(JSON.stringify(rows)),
      pageRows: page.length,
      pageSerializedBytes: Buffer.byteLength(JSON.stringify(page)),
      pageQueryMs,
    }),
  )
}
db.close()
