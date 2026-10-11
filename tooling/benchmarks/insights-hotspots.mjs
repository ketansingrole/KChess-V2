import { spawn } from 'node:child_process'
import { DatabaseSync } from 'node:sqlite'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import { cpus, hostname, platform, release, tmpdir, totalmem } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { performance } from 'node:perf_hooks'

const repo = fileURLToPath(new URL('../..', import.meta.url))
const baselineRoot = resolve(
  process.env.KCHESS_BENCHMARK_BASELINE_ROOT ?? '/private/tmp/kchess-benchmark-ts',
)
const baselineEntry = join(baselineRoot, 'core/dist/index.js')
const outputPath = resolve(
  process.env.KCHESS_INSIGHTS_HOTSPOTS_OUTPUT ??
    join(repo, 'docs/benchmarks/raw/insights-hotspots-2026-10.json'),
)
const repeats = 7
const GAME_COUNT = 50_000
const PAGE_QUERY = { account: 'benchmark', offset: 0, limit: 100 }
const INSIGHTS_QUERY = { account: 'benchmark', days: 3650 }

const SEARCH_FILTER = `account = ? AND createdAt >= ? AND status NOT IN ('created', 'started')`
const RESULT_SQL =
  "CASE WHEN winner IS NULL THEN 'draw' WHEN winner = color THEN 'win' ELSE 'loss' END"
const PLIES_SQL =
  "CASE WHEN moves = '' THEN 0 ELSE LENGTH(moves) - LENGTH(REPLACE(moves, ' ', '')) + 1 END"
const INSIGHTS_ORIGINAL = `SELECT id, color, speed, status, opening, opponentRating, playerRating, createdAt,
  (${RESULT_SQL}) AS result,
  CAST(strftime('%w', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS weekday,
  CAST(strftime('%H', createdAt / 1000, 'unixepoch', 'localtime') AS INTEGER) AS hour,
  ${PLIES_SQL} AS plies
FROM games WHERE ${SEARCH_FILTER} ORDER BY createdAt DESC LIMIT 20000`
const INSIGHTS_COMBINED = `WITH source AS MATERIALIZED (
  SELECT id, color, speed, status, opening, opponentRating, playerRating, createdAt,
    (${RESULT_SQL}) AS result,
    strftime('%w %H', createdAt / 1000, 'unixepoch', 'localtime') AS localParts,
    ${PLIES_SQL} AS plies
  FROM games WHERE ${SEARCH_FILTER} ORDER BY createdAt DESC LIMIT 20000
)
SELECT id, color, speed, status, opening, opponentRating, playerRating, createdAt, result,
  CAST(substr(localParts, 1, 1) AS INTEGER) AS weekday,
  CAST(substr(localParts, 3, 2) AS INTEGER) AS hour, plies
FROM source ORDER BY createdAt DESC`
const INSIGHTS_UTC = `SELECT id, color, speed, status, opening, opponentRating, playerRating, createdAt,
  (${RESULT_SQL}) AS result,
  CAST(strftime('%w', createdAt / 1000, 'unixepoch') AS INTEGER) AS weekday,
  CAST(strftime('%H', createdAt / 1000, 'unixepoch') AS INTEGER) AS hour,
  ${PLIES_SQL} AS plies
FROM games WHERE ${SEARCH_FILTER} ORDER BY createdAt DESC LIMIT 20000`
const INSIGHTS_RAW_DATE = `SELECT id, color, speed, status, opening, opponentRating, playerRating, createdAt,
  (${RESULT_SQL}) AS result, ${PLIES_SQL} AS plies
FROM games WHERE ${SEARCH_FILTER} ORDER BY createdAt DESC LIMIT 20000`
const INSIGHTS_HOUR_BUCKETS = `WITH source AS MATERIALIZED (
  SELECT id, color, speed, status, opening, opponentRating, playerRating, createdAt,
    (${RESULT_SQL}) AS result, CAST(createdAt / 3600000 AS INTEGER) AS utcHour,
    ${PLIES_SQL} AS plies
  FROM games WHERE ${SEARCH_FILTER} ORDER BY createdAt DESC LIMIT 20000
), hours AS MATERIALIZED (
  SELECT DISTINCT utcHour,
    strftime('%w %H', utcHour * 3600, 'unixepoch', 'localtime') AS localParts FROM source
)
SELECT source.id, source.color, source.speed, source.status, source.opening, source.opponentRating,
  source.playerRating, source.createdAt, source.result,
  CAST(substr(hours.localParts, 1, 1) AS INTEGER) AS weekday,
  CAST(substr(hours.localParts, 3, 2) AS INTEGER) AS hour, source.plies
FROM source JOIN hours USING (utcHour) ORDER BY source.createdAt DESC`
const LIST_COLUMNS =
  'account, id, createdAt, lastMoveAt, rated, speed, perf, status, winner, color, opponent, opponentRating, playerRating, ratingDiff, opening, moves'
const PAGE = `SELECT ${LIST_COLUMNS} FROM games WHERE account = ? COLLATE NOCASE ORDER BY createdAt DESC, account, id LIMIT ? OFFSET ?`
const PAGE_COUNT = 'SELECT COUNT(*) AS total FROM games WHERE account = ? COLLATE NOCASE'

const SQL_VARIANTS = {
  insights_original: {
    sql: INSIGHTS_ORIGINAL,
    params: () => [INSIGHTS_QUERY.account, Date.now() - INSIGHTS_QUERY.days * 86_400_000],
  },
  insights_combined_localtime: {
    sql: INSIGHTS_COMBINED,
    params: () => [INSIGHTS_QUERY.account, Date.now() - INSIGHTS_QUERY.days * 86_400_000],
  },
  insights_hour_bucket_cte: {
    sql: INSIGHTS_HOUR_BUCKETS,
    params: () => [INSIGHTS_QUERY.account, Date.now() - INSIGHTS_QUERY.days * 86_400_000],
  },
  insights_utc_diagnostic: {
    sql: INSIGHTS_UTC,
    params: () => [INSIGHTS_QUERY.account, Date.now() - INSIGHTS_QUERY.days * 86_400_000],
  },
  insights_raw_date_js_local: {
    sql: INSIGHTS_RAW_DATE,
    params: () => [INSIGHTS_QUERY.account, Date.now() - INSIGHTS_QUERY.days * 86_400_000],
    localDateMap: true,
  },
  gamePage_count: { sql: PAGE_COUNT, params: () => [PAGE_QUERY.account] },
  gamePage_rows: {
    sql: PAGE,
    params: () => [PAGE_QUERY.account, PAGE_QUERY.limit, PAGE_QUERY.offset],
  },
}

function seedGames(dataDir, count, openingCardinality = 2) {
  const db = new DatabaseSync(join(dataDir, 'kchess.db'))
  try {
    const insert = db.prepare(`INSERT INTO games
      (account, id, createdAt, lastMoveAt, rated, speed, perf, status, winner, color,
       opponent, opponentRating, playerRating, ratingDiff, opening, moves, pgn)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`)
    db.exec('BEGIN IMMEDIATE')
    for (let index = 0; index < count; index++) {
      const createdAt = 1_700_000_000_000 + index * 60_000
      const outcome = index % 3
      insert.run(
        'benchmark',
        `bench-${String(index).padStart(6, '0')}`,
        createdAt,
        createdAt + 180_000,
        index % 2,
        index % 4 === 0 ? 'rapid' : 'blitz',
        index % 4 === 0 ? 'rapid' : 'blitz',
        'mate',
        outcome === 2 ? null : outcome === 0 ? 'white' : 'black',
        index % 2 === 0 ? 'white' : 'black',
        `Opponent ${index % 1000}`,
        1200 + (index % 1200),
        1250 + (index % 1000),
        (index % 41) - 20,
        `Opening ${index % openingCardinality}`,
        'e2e4 e7e5 g1f3 b8c6 f1c4',
        `[Event "Benchmark"]\n[Date "2026.01.01"]\n[Result "*"]\n\n1. e4 e5 2. Nf3 Nc6 3. Bc4 *`,
      )
    }
    db.exec('COMMIT')
  } catch (error) {
    db.exec('ROLLBACK')
    throw error
  } finally {
    db.close()
  }
}

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical)
  if (!value || typeof value !== 'object') return value
  return Object.fromEntries(
    Object.keys(value)
      .sort()
      .map((key) => [key, canonical(value[key])]),
  )
}

function summarizeReport(report) {
  return {
    total: report.total,
    record: report.record,
    byColor: report.byColor,
    bySpeed: report.bySpeed,
    byOpening: report.byOpening,
    byWeekday: report.byWeekday,
    byHour: report.byHour,
    byOpponent: report.byOpponent,
    byLength: report.byLength,
    endings: report.endings,
    streaks: report.streaks,
  }
}

function timeSamples(samples, name, work, count = repeats) {
  return (async () => {
    const values = []
    let last
    for (let i = 0; i < count; i++) {
      const start = performance.now()
      last = await work()
      values.push(performance.now() - start)
    }
    samples[name] = values
    return last
  })()
}

async function worker(implementation) {
  const dataDir = await import('node:fs/promises').then(({ mkdtemp }) =>
    mkdtemp(join(tmpdir(), `kchess-insights-${implementation}-`)),
  )
  await writeFile(join(dataDir, 'kchess-data.json'), '{}\n')
  await writeFile(join(dataDir, 'lichess-tokens.json'), '{}\n')
  let core
  let createKChessCore
  let bundledEnginePath
  let nativeCall
  let nativeCallSync
  const samples = {}
  const plans = {}
  const signatures = {}
  const explainDb = new DatabaseSync(':memory:')
  try {
    if (implementation === 'typescript') {
      const baseline = await import(pathToFileURL(baselineEntry).href)
      createKChessCore = baseline.createKChessCore
      bundledEnginePath = join(baselineRoot, baseline.BUNDLED_ENGINE_SCRIPT)
    } else {
      const current = await import(
        pathToFileURL(join(repo, 'crates/kchess-node/dist/index.js')).href
      )
      ;({ createKChessCore, BUNDLED_ENGINE_SCRIPT: bundledEnginePath } = current)
      bundledEnginePath = join(repo, bundledEnginePath)
      const bridge = await import(
        pathToFileURL(join(repo, 'crates/kchess-node/dist/nativeCore.js')).href
      )
      ;({ nativeCall, nativeCallSync } = bridge)
    }
    const host = {
      dataDir,
      bundledEnginePath,
      managedEngineDir: join(dataDir, 'engines'),
      secrets: {
        available: () => false,
        encrypt: () => {
          throw new Error('unavailable')
        },
        decrypt: () => {
          throw new Error('unavailable')
        },
      },
      openExternal: async () => {},
      onBattery: () => false,
    }
    core = createKChessCore(host)
    await core.loadData()
    await core.close()
    core = undefined
    seedGames(dataDir, GAME_COUNT)
    core = createKChessCore(host)
    await core.loadData()

    const db = new DatabaseSync(join(dataDir, 'kchess.db'))
    const explain = (sql, params = []) => db.prepare(`EXPLAIN QUERY PLAN ${sql}`).all(...params)
    for (const [name, variant] of Object.entries(SQL_VARIANTS)) {
      const params = variant.params()
      plans[name] = explain(variant.sql, params)
    }
    plans.gamePage_count_and_rows = {
      count: explain(PAGE_COUNT, [PAGE_QUERY.account]),
      rows: explain(PAGE, [PAGE_QUERY.account, PAGE_QUERY.limit, PAGE_QUERY.offset]),
      resultFiltered: explain(
        `SELECT COUNT(*) FROM games WHERE account = ? COLLATE NOCASE AND (${RESULT_SQL}) = ?`,
        [PAGE_QUERY.account, 'win'],
      ),
    }

    const sqlResults = {}
    for (const [name, variant] of Object.entries(SQL_VARIANTS)) {
      if (name === 'gamePage_count' || name === 'gamePage_rows') continue
      const statement = db.prepare(variant.sql)
      const result = await timeSamples(samples, name, () => {
        const rows = statement.all(...variant.params())
        if (variant.localDateMap) {
          for (const row of rows) {
            const date = new Date(row.createdAt)
            row.weekday = date.getDay()
            row.hour = date.getHours()
          }
        }
        return rows
      })
      sqlResults[name] = result
    }
    await timeSamples(samples, 'gamePage_count_only', () =>
      db.prepare(PAGE_COUNT).get(PAGE_QUERY.account),
    )
    await timeSamples(samples, 'gamePage_rows_only', () =>
      db.prepare(PAGE).all(PAGE_QUERY.account, PAGE_QUERY.limit, PAGE_QUERY.offset),
    )
    await timeSamples(samples, 'gamePage_count_plus_rows', () => {
      const total = db.prepare(PAGE_COUNT).get(PAGE_QUERY.account).total
      const games = db.prepare(PAGE).all(PAGE_QUERY.account, PAGE_QUERY.limit, PAGE_QUERY.offset)
      return { total, games }
    })
    const rawCanonical = canonical(sqlResults.insights_original)
    for (const name of [
      'insights_combined_localtime',
      'insights_hour_bucket_cte',
      'insights_raw_date_js_local',
    ]) {
      signatures[name] =
        JSON.stringify(canonical(sqlResults[name])) === JSON.stringify(rawCanonical)
    }
    signatures.utcEquivalent =
      JSON.stringify(canonical(sqlResults.insights_utc_diagnostic)) === JSON.stringify(rawCanonical)
    signatures.sqlRows = sqlResults.insights_original.length
    signatures.uniqueUtcHours = new Set(
      sqlResults.insights_original.map((row) => Math.floor(row.createdAt / 3_600_000)),
    ).size
    signatures.localTimeZone = Intl.DateTimeFormat().resolvedOptions().timeZone

    const publicInsights = await timeSamples(samples, 'core_insights', () =>
      core.insights(INSIGHTS_QUERY),
    )
    signatures.publicInsights = summarizeReport(publicInsights)
    const publicPage = await timeSamples(samples, 'core_gamePage', () => core.gamePage(PAGE_QUERY))
    signatures.publicPage = {
      total: publicPage.total,
      ids: publicPage.games.map((game) => game.id),
    }
    if (implementation === 'rust') {
      const asyncStoreInsights = await timeSamples(samples, 'native_store_insights_async', () =>
        nativeCall('store.insights.insights', INSIGHTS_QUERY),
      )
      const syncStoreInsights = await timeSamples(samples, 'native_store_insights_sync', () =>
        nativeCallSync('store.insights.insights', INSIGHTS_QUERY),
      )
      signatures.storeInsightsAsyncParity =
        JSON.stringify(canonical(summarizeReport(asyncStoreInsights))) ===
        JSON.stringify(canonical(signatures.publicInsights))
      signatures.storeInsightsSyncParity =
        JSON.stringify(canonical(summarizeReport(syncStoreInsights))) ===
        JSON.stringify(canonical(signatures.publicInsights))
      const asyncPage = await timeSamples(samples, 'native_store_gamePage_async', () =>
        nativeCall('store.games.gamePage', PAGE_QUERY),
      )
      const syncPage = await timeSamples(samples, 'native_store_gamePage_sync', () =>
        nativeCallSync('store.games.gamePage', PAGE_QUERY),
      )
      signatures.storePageAsyncParity =
        JSON.stringify(canonical(asyncPage)) === JSON.stringify(canonical(publicPage))
      signatures.storePageSyncParity =
        JSON.stringify(canonical(syncPage)) === JSON.stringify(canonical(publicPage))
    }
    db.close()

    // Probe the Rust Vec::position opening-family lookup at bounded higher cardinality without
    // changing production code. TypeScript Maps retain O(1) lookup for this aggregation.
    if (implementation === 'rust') {
      await core.close()
      core = undefined
      const update = new DatabaseSync(join(dataDir, 'kchess.db'))
      update
        .prepare("UPDATE games SET opening = 'Opening ' || id WHERE account = 'benchmark'")
        .run()
      update.close()
      core = createKChessCore(host)
      await core.loadData()
    } else {
      const update = new DatabaseSync(join(dataDir, 'kchess.db'))
      update
        .prepare("UPDATE games SET opening = 'Opening ' || id WHERE account = 'benchmark'")
        .run()
      update.close()
    }
    const highCardinality = await timeSamples(
      samples,
      'core_insights_20000_openings',
      () => core.insights(INSIGHTS_QUERY),
      3,
    )
    signatures.highCardinalityOpenedGroups = highCardinality.byOpening.length
    signatures.highCardinalityTotal = highCardinality.total

    console.log(
      JSON.stringify({
        implementation,
        fixture: {
          games: GAME_COUNT,
          account: 'benchmark',
          createdAtStrideMs: 60_000,
          defaultOpeningCardinality: 2,
        },
        samples,
        plans,
        signatures,
      }),
    )
  } finally {
    explainDb.close()
    if (core) await core.close()
    await rm(dataDir, { recursive: true, force: true })
  }
}

async function runWorker(implementation) {
  const child = spawn(
    process.execPath,
    [fileURLToPath(import.meta.url), '--worker', implementation],
    {
      cwd: repo,
      env: { ...process.env, TZ: 'Asia/Tokyo' },
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  )
  let stdout = ''
  let stderr = ''
  child.stdout.setEncoding('utf8').on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.setEncoding('utf8').on('data', (chunk) => {
    stderr += chunk
  })
  const status = await new Promise((resolvePromise, reject) => {
    child.once('error', reject)
    child.once('close', (code, signal) => resolvePromise({ code, signal }))
  })
  if (status.code !== 0)
    throw new Error(
      `${implementation} insights-hotspot worker failed (${status.signal ?? status.code})\n${stderr}\n${stdout}`,
    )
  const line = stdout
    .trim()
    .split('\n')
    .reverse()
    .find((row) => row.trim().startsWith('{'))
  if (!line)
    throw new Error(`Missing ${implementation} insights worker JSON:\n${stderr}\n${stdout}`)
  return { ...JSON.parse(line), stderr: stderr.trim() || undefined }
}

function percentile(values, fraction) {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.ceil(fraction * sorted.length) - 1)] ?? null
}

if (process.argv[2] === '--worker') {
  await worker(process.argv[3])
} else {
  const implementations = ['typescript', 'rust']
  const runs = []
  for (const implementation of implementations) {
    console.error(`Insights hotspot study: ${implementation}`)
    runs.push(await runWorker(implementation))
  }
  const sampleNames = [...new Set(runs.flatMap((run) => Object.keys(run.samples)))].sort()
  const aggregates = Object.fromEntries(
    sampleNames.map((name) => [
      name,
      Object.fromEntries(
        runs.map((run) => [
          run.implementation,
          {
            n: run.samples[name]?.length ?? 0,
            p50Ms: run.samples[name] ? percentile(run.samples[name], 0.5) : null,
            p95Ms: run.samples[name] ? percentile(run.samples[name], 0.95) : null,
            samplesMs: run.samples[name] ?? [],
          },
        ]),
      ),
    ]),
  )
  const result = {
    schemaVersion: 1,
    title: 'KChess insights and gamePage hotspot probes',
    capturedAt: new Date().toISOString(),
    revisions: { rust: '8f58da7', typescript: 'f479f7c' },
    environment: {
      hostname: hostname(),
      platform: `${platform()} ${release()}`,
      node: process.version,
      cpu: cpus()[0]?.model,
      logicalCpuCount: cpus().length,
      totalMemoryBytes: totalmem(),
      timezone: 'Asia/Tokyo',
    },
    methodology: {
      fixture:
        'The 50k-game fixture matches the core study: one account, 60 s timestamp stride, rated/speed/result distributions, 5-ply move strings, two repeated openings. The high-cardinality probe changes only opening labels to 20,000 distinct values on the scratch DB.',
      repeats: `${repeats} warm samples per SQL and public/native method, except three bounded high-cardinality API samples. TypeScript and Rust run in separate sequential workers on identical fixtures; these are microbenchmarks, not confidence intervals over independent processes.`,
      sql: 'SQLite statements mirror insights.rs/insights.ts and games.rs; EXPLAIN QUERY PLAN results are included. SQL variants preserve all selected data except the UTC diagnostic, which intentionally changes local-time semantics. Combined-localtime and hour-bucket CTE variants are accepted only if their result rows match original SQL for every fixture row.',
      page: 'Measures COUNT only, page rows only, and the two queries together. The core study showed TypeScript caches filtered totals after the first call while Rust counts every time.',
      napi: 'Rust also times the same store.insights and store.games.gamePage methods through NativeCore.call (async) and NativeCore.callSync (sync), bypassing only the CoreApi forwarding closure; returned output parity is checked.',
      boundaries:
        'This measures wall latency only; it does not claim a direct CPU attribution between SQLite, Rust aggregation, JSON serialization, and N-API scheduling. EXPLAIN plans identify query strategy but not time by node.',
      timeZone:
        'The verified host/system zone was Asia/Tokyo; workers set TZ=Asia/Tokyo. UTC rows are diagnostic only and are expected to differ for some timestamps.',
      scratchOnly:
        'Proposed SQL is executed only against temporary fixture profiles. No product source or schema is changed.',
    },
    aggregates,
    runs,
  }
  await mkdir(join(repo, 'docs/benchmarks/raw'), { recursive: true })
  await writeFile(outputPath, `${JSON.stringify(result, null, 2)}\n`)
  console.log(`Wrote ${outputPath}`)
}
