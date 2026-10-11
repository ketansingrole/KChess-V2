import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { DatabaseSync } from 'node:sqlite'
import { mkdtemp, rm } from 'node:fs/promises'
import { cpus, hostname, platform, release, tmpdir, totalmem } from 'node:os'
import { realpathSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { performance } from 'node:perf_hooks'

const scriptPath = fileURLToPath(import.meta.url)
const fixtureGames = 50_000
const queryLimit = 20_000
const edgeStart = Date.UTC(1948, 0, 1)
const edgeEnd = Date.UTC(1953, 0, 1)
const edgeStride = 30 * 60_000
const repeats = 7
const createdAtStart = 1_700_000_000_000
const createdAtStride = 60_000
const dayMs = 86_400_000

const BASE = `SELECT id, createdAt,
  CAST(strftime('%w', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS weekday,
  CAST(strftime('%H', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS hour
FROM games WHERE account = ? AND createdAt >= ? AND status NOT IN ('created', 'started')
ORDER BY createdAt DESC LIMIT ${queryLimit}`
const UTC_HOUR =
  'CAST(createdAt / 3600000 AS INTEGER) - CASE WHEN createdAt < 0 AND createdAt % 3600000 != 0 THEN 1 ELSE 0 END'
const COMBINED = `WITH source AS MATERIALIZED (
  SELECT id, createdAt,
    strftime('%w %H', createdAt / 1000, 'unixepoch', 'localtime') AS localParts
  FROM games WHERE account = ? AND createdAt >= ? AND status NOT IN ('created', 'started')
  ORDER BY createdAt DESC LIMIT ${queryLimit}
)
SELECT id, createdAt,
  CAST(substr(localParts, 1, 1) AS INTEGER) AS weekday,
  CAST(substr(localParts, 3, 2) AS INTEGER) AS hour
FROM source ORDER BY createdAt DESC`
const HOUR_BUCKETED = `WITH source AS MATERIALIZED (
  SELECT id, createdAt, ${UTC_HOUR} AS utcHour
  FROM games WHERE account = ? AND createdAt >= ? AND status NOT IN ('created', 'started')
  ORDER BY createdAt DESC LIMIT ${queryLimit}
), unique_hours AS MATERIALIZED (
  SELECT DISTINCT utcHour FROM source
), hours AS MATERIALIZED (
  SELECT utcHour,
    strftime('%w %H', utcHour * 3600, 'unixepoch', 'localtime') AS localParts FROM unique_hours
)
SELECT source.id, source.createdAt,
  CAST(substr(hours.localParts, 1, 1) AS INTEGER) AS weekday,
  CAST(substr(hours.localParts, 3, 2) AS INTEGER) AS hour
FROM source JOIN hours USING (utcHour) ORDER BY source.createdAt DESC`

function digest(rows) {
  const hash = createHash('sha256')
  for (const row of rows) hash.update(`${row.id}\t${row.createdAt}\t${row.weekday}\t${row.hour}\n`)
  return hash.digest('hex')
}

function percentile(values, fraction) {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.ceil(fraction * sorted.length) - 1)] ?? null
}

async function worker(label) {
  const root = await mkdtemp(join(tmpdir(), 'kchess-insights-tz-'))
  const dbPath = join(root, 'fixture.sqlite')
  const db = new DatabaseSync(dbPath)
  try {
    db.exec(`CREATE TABLE games (
      account TEXT NOT NULL, id TEXT NOT NULL, createdAt INTEGER NOT NULL,
      status TEXT NOT NULL, color TEXT NOT NULL, winner TEXT, opening TEXT,
      opponentRating INTEGER, playerRating INTEGER, speed TEXT NOT NULL,
      moves TEXT NOT NULL, PRIMARY KEY(account, id));
      CREATE INDEX idx_games_account_created ON games(account, createdAt DESC);`)
    const insert = db.prepare(`INSERT INTO games
      (account, id, createdAt, status, color, winner, opening, opponentRating, playerRating, speed, moves)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`)
    db.exec('BEGIN IMMEDIATE')
    for (let i = 0; i < fixtureGames; i++) {
      const outcome = i % 3
      const createdAt = createdAtStart + i * createdAtStride
      insert.run(
        'benchmark',
        `bench-${String(i).padStart(6, '0')}`,
        createdAt,
        'mate',
        i % 2 === 0 ? 'white' : 'black',
        outcome === 2 ? null : outcome === 0 ? 'white' : 'black',
        `Opening ${i % 2}`,
        1200 + (i % 1200),
        1250 + (i % 1000),
        i % 4 === 0 ? 'rapid' : 'blitz',
        'e2e4 e7e5 g1f3 b8c6 f1c4',
      )
    }
    // Tokyo had seasonal DST in the late 1940s/early 1950s. Sample every 30 minutes across
    // those transitions as well as the modern fixture, to catch timezone-source mismatches.
    for (let timestamp = edgeStart; timestamp < edgeEnd; timestamp += edgeStride) {
      insert.run(
        'tz-edge',
        `edge-${timestamp}`,
        timestamp,
        'mate',
        'white',
        'white',
        'Historical zone check',
        1200,
        1250,
        'blitz',
        'e2e4 e7e5',
      )
    }
    db.exec('COMMIT')

    const params = ['benchmark', Date.now() - 3650 * dayMs]
    const query = Object.fromEntries(
      Object.entries({
        original: BASE,
        combinedLocaltime: COMBINED,
        hourBucketed: HOUR_BUCKETED,
      }).map(([name, sql]) => [name, db.prepare(sql)]),
    )
    const rows = Object.fromEntries(
      Object.keys(query).map((name) => [name, query[name].all(...params)]),
    )
    const samples = Object.fromEntries(Object.keys(query).map((name) => [name, []]))
    // Warm each statement and SQLite's page cache before capturing process CPU deltas.
    for (let i = 0; i < 2; i++) for (const name of Object.keys(query)) query[name].all(...params)
    const cpuStart = process.resourceUsage()
    const wallStart = performance.now()
    for (let i = 0; i < repeats; i++) {
      for (const name of Object.keys(query)) {
        const start = performance.now()
        query[name].all(...params)
        samples[name].push(performance.now() - start)
      }
    }
    const wallMs = performance.now() - wallStart
    const cpuEnd = process.resourceUsage()
    const signatures = Object.fromEntries(
      Object.entries(rows).map(([name, data]) => [name, digest(data)]),
    )
    const edgeLimit = 100_000
    const edgeParams = ['tz-edge', edgeStart]
    const edgeQueries = Object.fromEntries(
      Object.entries({
        original: BASE,
        combinedLocaltime: COMBINED,
        hourBucketed: HOUR_BUCKETED,
      }).map(([name, sql]) => [
        name,
        db.prepare(sql.replaceAll(`LIMIT ${queryLimit}`, `LIMIT ${edgeLimit}`)),
      ]),
    )
    const edgeRows = Object.fromEntries(
      Object.entries(edgeQueries).map(([name, statement]) => [name, statement.all(...edgeParams)]),
    )
    const edgeSignatures = Object.fromEntries(
      Object.entries(edgeRows).map(([name, data]) => [name, digest(data)]),
    )
    console.log(
      JSON.stringify({
        label,
        processTz: process.env.TZ ?? null,
        hostResolvedZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        samplesMs: samples,
        summariesMs: Object.fromEntries(
          Object.entries(samples).map(([name, data]) => [
            name,
            { p50: percentile(data, 0.5), p95: percentile(data, 0.95) },
          ]),
        ),
        cpu: {
          userMs: (cpuEnd.userCPUTime - cpuStart.userCPUTime) / 1000,
          systemMs: (cpuEnd.systemCPUTime - cpuStart.systemCPUTime) / 1000,
          wallMs,
        },
        output: {
          rows: Object.fromEntries(Object.entries(rows).map(([name, data]) => [name, data.length])),
          sha256: signatures,
          historicalDst: {
            rows: Object.fromEntries(
              Object.entries(edgeRows).map(([name, data]) => [name, data.length]),
            ),
            sha256: edgeSignatures,
          },
        },
        uniqueUtcHours: new Set(rows.original.map((row) => Math.floor(row.createdAt / 3_600_000)))
          .size,
      }),
    )
  } finally {
    db.close()
    await rm(root, { recursive: true, force: true })
  }
}

function runWorker(label, tz) {
  const env = { ...process.env }
  if (tz === null) delete env.TZ
  else env.TZ = tz
  const child = spawn(process.execPath, [scriptPath, '--worker', label], {
    cwd: process.cwd(),
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8').on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.setEncoding('utf8').on('data', (chunk) => {
    stderr += chunk
  })
  return new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('close', (code, signal) => {
      if (code !== 0)
        return reject(
          new Error(`Timezone worker ${label} failed (${signal ?? code})\n${stderr}\n${stdout}`),
        )
      const line = stdout
        .trim()
        .split('\n')
        .reverse()
        .find((row) => row.startsWith('{'))
      if (!line)
        return reject(new Error(`Timezone worker ${label} emitted no JSON\n${stderr}\n${stdout}`))
      resolve(JSON.parse(line))
    })
  })
}

if (process.argv[2] === '--worker') {
  await worker(process.argv[3])
} else {
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone
  const localtimePath = realpathSync('/etc/localtime')
  const variants = [
    ['host-default-unset', null],
    ['explicit-zone-name', zone],
    ['explicit-localtime-file', `:${localtimePath}`],
  ]
  const runs = []
  for (const [label, tz] of variants) {
    console.error(`Timezone probe: ${label} (${tz ?? 'TZ unset'})`)
    runs.push(await runWorker(label, tz))
  }
  const comparable = runs.map((run) => run.output.sha256)
  const result = {
    schemaVersion: 1,
    title: 'KChess insights timezone evaluation probe',
    capturedAt: new Date().toISOString(),
    environment: {
      hostname: hostname(),
      platform: `${platform()} ${release()}`,
      node: process.version,
      cpu: cpus()[0]?.model,
      logicalCpuCount: cpus().length,
      totalMemoryBytes: totalmem(),
      actualHostZone: zone,
      localtimeRealpath: localtimePath,
    },
    methodology: {
      fixture: `${fixtureGames} deterministic game rows matching the core insights column distribution and timestamp stride; each process builds its own identical scratch SQLite DB before timing.`,
      query: `The insights query selects the newest ${queryLimit} eligible rows and returns local weekday/hour using either two localtime expressions, one combined expression, or one per distinct UTC hour.`,
      timing: `${repeats} warm rounds per SQL variant in each of three sequential fresh processes. Process CPU deltas are sampled after DB creation, first query warm-up, and SQLite cache warm-up; they include all timed SQL variants.`,
      timezone:
        'TZ is unset for the default run, then set in separate child processes to the host-resolved zone name and to the exact /etc/localtime file. This only affects benchmark subprocesses.',
      limits:
        'Wall and process user/system CPU deltas do not prove an individual syscall cause. The zone behavior is checked by full result-row digests, not by assumptions about names.',
    },
    outputParityAcrossTimezoneVariants: comparable.every(
      (signatureSet) => JSON.stringify(signatureSet) === JSON.stringify(comparable[0]),
    ),
    historicalDstParityAcrossTimezoneVariants:
      runs.every((run) => {
        const signatures = run.output.historicalDst.sha256
        return Object.values(signatures).every((signature) => signature === signatures.original)
      }) &&
      runs.every(
        (run) =>
          JSON.stringify(run.output.historicalDst.sha256) ===
          JSON.stringify(runs[0].output.historicalDst.sha256),
      ),
    runs,
  }
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`)
}
