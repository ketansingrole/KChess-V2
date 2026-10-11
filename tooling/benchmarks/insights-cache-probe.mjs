#!/usr/bin/env node
// SQL-only evaluation of exact-timestamp local-calendar reuse. This isolates the query shape;
// it does not include the Rust aggregator or claim the synthetic timestamp mixes are user data.
import { createHash } from 'node:crypto'
import { DatabaseSync } from 'node:sqlite'
import { performance } from 'node:perf_hooks'
import { writeFileSync } from 'node:fs'

const ROWS = 20_000
const PAIRS = 7
const START = Date.UTC(2026, 0, 1)
const ORIGINAL = `WITH source AS MATERIALIZED (
  SELECT id, createdAt,
    strftime('%w %H', createdAt / 1000, 'unixepoch', 'localtime') AS local_parts
  FROM games WHERE account = 'Alice' AND status NOT IN ('created', 'started')
  ORDER BY createdAt DESC LIMIT ${ROWS}
)
SELECT id, createdAt,
  CAST(substr(local_parts, 1, 1) AS INTEGER) AS weekday,
  CAST(substr(local_parts, 3, 2) AS INTEGER) AS hour
FROM source ORDER BY createdAt DESC`
const EXACT_CACHE = `WITH source AS MATERIALIZED (
  SELECT id, createdAt
  FROM games WHERE account = 'Alice' AND status NOT IN ('created', 'started')
  ORDER BY createdAt DESC LIMIT ${ROWS}
), unique_times AS MATERIALIZED (
  SELECT DISTINCT createdAt FROM source
), local_times AS MATERIALIZED (
  SELECT createdAt,
    strftime('%w %H', createdAt / 1000, 'unixepoch', 'localtime') AS local_parts
  FROM unique_times
)
SELECT source.id, source.createdAt,
  CAST(substr(local_times.local_parts, 1, 1) AS INTEGER) AS weekday,
  CAST(substr(local_times.local_parts, 3, 2) AS INTEGER) AS hour
FROM source JOIN local_times USING (createdAt)
ORDER BY source.createdAt DESC`

const digest = (rows) => {
  const hash = createHash('sha256')
  for (const row of rows) hash.update(`${row.id}\t${row.createdAt}\t${row.weekday}\t${row.hour}\n`)
  return hash.digest('hex')
}
const percentile = (values, fraction) => {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.ceil(fraction * sorted.length) - 1)] ?? null
}

function timestamps(kind) {
  return Array.from({ length: ROWS }, (_, index) => {
    if (kind === 'unique') return START + index * 60_000
    if (kind === 'duplicates') return START + Math.floor(index / 200) * 60_000
    // Synthetic mixed workload: 16k unique instants plus 4k rows reusing those instants.
    if (kind === 'mixed-80-percent-unique')
      return START + (index < 16_000 ? index : index - 16_000) * 60_000
    throw new Error(`Unknown fixture: ${kind}`)
  })
}

function fixture(kind) {
  const db = new DatabaseSync(':memory:')
  db.exec(`CREATE TABLE games (
    account TEXT NOT NULL, id TEXT NOT NULL, createdAt INTEGER NOT NULL,
    status TEXT NOT NULL, PRIMARY KEY(account, id));
    CREATE INDEX idx_games_account_created ON games(account, createdAt DESC);`)
  const insert = db.prepare('INSERT INTO games VALUES (?, ?, ?, ?)')
  const values = timestamps(kind)
  db.exec('BEGIN')
  for (let index = 0; index < values.length; index++)
    insert.run('Alice', `g${String(index).padStart(5, '0')}`, values[index], 'mate')
  db.exec('COMMIT')
  return { db, distinctTimestamps: new Set(values).size }
}

function measure(db, sql) {
  const start = performance.now()
  const rows = db.prepare(sql).all()
  return { elapsedMs: performance.now() - start, rows, digest: digest(rows) }
}

const fixtures = []
for (const kind of ['unique', 'duplicates', 'mixed-80-percent-unique']) {
  const { db, distinctTimestamps } = fixture(kind)
  try {
    const parity = Object.fromEntries(
      [
        ['original', ORIGINAL],
        ['exactCache', EXACT_CACHE],
      ].map(([name, sql]) => {
        const result = measure(db, sql)
        return [name, { rows: result.rows.length, digest: result.digest }]
      }),
    )
    if (parity.original.rows !== ROWS || parity.original.digest !== parity.exactCache.digest)
      throw new Error(`${kind}: exact cache changed row count or order: ${JSON.stringify(parity)}`)

    const samples = { original: [], exactCache: [] }
    for (let pair = 0; pair < PAIRS; pair++) {
      const order = pair % 2 === 0 ? ['original', 'exactCache'] : ['exactCache', 'original']
      for (const name of order) {
        const result = measure(db, name === 'original' ? ORIGINAL : EXACT_CACHE)
        if (result.digest !== parity.original.digest)
          throw new Error(`${kind} ${name}: output changed during timed pair ${pair + 1}`)
        samples[name].push(result.elapsedMs)
      }
    }
    fixtures.push({
      kind,
      inputRows: ROWS,
      uniqueTimestamps: distinctTimestamps,
      output: parity,
      samplesMs: samples,
      summariesMs: Object.fromEntries(
        Object.entries(samples).map(([name, values]) => [
          name,
          {
            p50: percentile(values, 0.5),
            p95: percentile(values, 0.95),
          },
        ]),
      ),
    })
  } finally {
    db.close()
  }
}

const output = {
  kind: 'insights-exact-timestamp-cache-sql-probe',
  node: process.version,
  sqlite: new DatabaseSync(':memory:').prepare('SELECT sqlite_version() AS version').get().version,
  os: process.platform,
  arch: process.arch,
  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  rowsPerFixture: ROWS,
  counterbalancedPairs: PAIRS,
  queryLimit: ROWS,
  originalQuery: 'One combined localtime strftime per selected row.',
  candidateQuery:
    'Materialized exact DISTINCT createdAt values; one localtime strftime per unique timestamp; join back to selected rows.',
  fixtureDefinitions: {
    unique: '20,000 timestamps at one-minute intervals.',
    duplicates: '100 timestamps, each repeated 200 times.',
    'mixed-80-percent-unique':
      '16,000 unique timestamps plus 4,000 rows reusing those exact values.',
  },
  fixtures,
}
const destination = process.argv[2]
if (!destination)
  throw new Error('Usage: node tooling/benchmarks/insights-cache-probe.mjs <raw-output.json>')
writeFileSync(destination, `${JSON.stringify(output, null, 2)}\n`)
console.log(JSON.stringify(output, null, 2))
