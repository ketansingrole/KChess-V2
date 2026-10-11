import { spawn } from 'node:child_process'
import { DatabaseSync } from 'node:sqlite'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import { arch, cpus, hostname, platform, release, tmpdir, totalmem } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { monitorEventLoopDelay, performance } from 'node:perf_hooks'

const projectRoot = fileURLToPath(new URL('..', import.meta.url))
const baselineRoot = resolve(
  process.env.KCHESS_BENCHMARK_BASELINE_ROOT ?? '/private/tmp/kchess-benchmark-ts',
)
const baselineEntry = join(baselineRoot, 'core/dist/index.js')
const outputPath = resolve(
  process.env.KCHESS_PUZZLE_BENCHMARK_OUTPUT ??
    join(projectRoot, 'docs/benchmarks/raw/core-puzzles-2026-10.json'),
)
const counts = [10_000, 100_000]
const pairs = 3

function seedPuzzles(dataDir, count) {
  const db = new DatabaseSync(join(dataDir, 'puzzles.db'))
  try {
    db.exec(`
      PRAGMA journal_mode = WAL;
      CREATE TABLE IF NOT EXISTS puzzles (id TEXT PRIMARY KEY, fen TEXT NOT NULL, moves TEXT NOT NULL, rating INTEGER NOT NULL, plays INTEGER NOT NULL DEFAULT 0, themes TEXT NOT NULL);
      CREATE INDEX IF NOT EXISTS idx_puzzles_rating ON puzzles(rating);
      CREATE TABLE IF NOT EXISTS puzzle_meta (id INTEGER PRIMARY KEY CHECK(id = 1), importedAt INTEGER NOT NULL, count INTEGER NOT NULL, bytes INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS worker_meta (id INTEGER PRIMARY KEY CHECK(id = 1));
      DELETE FROM puzzles;
      DELETE FROM puzzle_meta;
    `)
    const insert = db.prepare(
      'INSERT INTO puzzles (id, fen, moves, rating, plays, themes) VALUES (?, ?, ?, ?, ?, ?)',
    )
    const txStart = performance.now()
    db.exec('BEGIN IMMEDIATE')
    for (let index = 0; index < count; index++) {
      const id = `fixture-${String(index).padStart(6, '0')}`
      const themes = index % 2 === 0 ? 'mateIn2 middlegame' : 'opening'
      insert.run(
        id,
        'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1',
        'e2e4 e7e5',
        800 + (index % 1000),
        index % 13,
        themes,
      )
    }
    const bytes = db
      .prepare(
        'SELECT COALESCE(SUM(LENGTH(id) + LENGTH(fen) + LENGTH(moves) + LENGTH(themes) + 24), 0) AS bytes FROM puzzles',
      )
      .get().bytes
    db.prepare('INSERT INTO puzzle_meta (id, importedAt, count, bytes) VALUES (1, ?, ?, ?)').run(
      1_791_600_000_000,
      count,
      bytes,
    )
    db.exec('COMMIT')
    return { seedElapsedMs: performance.now() - txStart, bytes }
  } catch (error) {
    db.exec('ROLLBACK')
    throw error
  } finally {
    db.close()
  }
}

async function worker(implementation) {
  const dataDir = join(tmpdir(), `kchess-puzzles-${process.pid}-${implementation}`)
  await mkdir(dataDir, { recursive: true })
  await writeFile(join(dataDir, 'kchess-data.json'), '{}\n')
  await writeFile(join(dataDir, 'lichess-tokens.json'), '{}\n')
  let core
  const loopDelay = monitorEventLoopDelay({ resolution: 10 })
  loopDelay.enable()
  const samples = {}
  const signatures = {}
  const fixture = {}
  const measure = async (name, work) => {
    const started = performance.now()
    const result = await work()
    ;(samples[name] ??= []).push(performance.now() - started)
    return result
  }
  try {
    let createKChessCore
    let bundledEnginePath
    if (implementation === 'typescript') {
      const baseline = await import(pathToFileURL(baselineEntry).href)
      createKChessCore = baseline.createKChessCore
      bundledEnginePath = join(baselineRoot, baseline.BUNDLED_ENGINE_SCRIPT)
    } else {
      const current = await import(
        pathToFileURL(join(projectRoot, 'crates/kchess-node/dist/index.js')).href
      )
      createKChessCore = current.createKChessCore
      bundledEnginePath = join(projectRoot, current.BUNDLED_ENGINE_SCRIPT)
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

    for (const count of counts) {
      if (!core) {
        core = createKChessCore(host)
        await core.loadData()
      }
      await core.puzzleDbStatus() // Creates the service-owned database and its schema.
      await core.close()
      core = undefined
      const fixtureTiming = seedPuzzles(dataDir, count)
      core = createKChessCore(host)
      await core.loadData()
      const status = await measure(`puzzles_${count}_status`, () => core.puzzleDbStatus())
      const query = { theme: 'mateIn2', minRating: 900, maxRating: 1600, count: 20 }
      const ladderQuery = { from: 800, to: 1800, count: 10 }
      let validQuery = true
      let validLadder = true
      for (let index = 0; index < 20; index++) {
        const puzzles = await measure(`puzzles_${count}_localPuzzles`, () =>
          core.localPuzzles(query),
        )
        const ladder = await measure(`puzzles_${count}_localLadder`, () =>
          core.localLadder(ladderQuery),
        )
        validQuery &&=
          puzzles.length === query.count &&
          puzzles.every(
            (item) =>
              item.themes.includes('mateIn2') &&
              item.rating >= query.minRating &&
              item.rating <= query.maxRating &&
              item.solution.length === 1 &&
              item.solution[0] === 'e7e5' &&
              /^[A-Za-z0-9_-]+$/.test(item.id),
          )
        validLadder &&=
          ladder.length === ladderQuery.count &&
          ladder.every(
            (item) =>
              item.solution.length === 1 &&
              item.solution[0] === 'e7e5' &&
              Number.isFinite(item.rating) &&
              item.rating >= ladderQuery.from &&
              item.rating <= ladderQuery.to,
          )
      }
      fixture[count] = fixtureTiming
      signatures[count] = { status, validQuery, validLadder }
      await core.close()
      core = undefined
    }
    const usage = process.resourceUsage()
    console.log(
      JSON.stringify({
        implementation,
        counts,
        samples,
        signatures,
        fixture,
        memory: {
          rss: process.memoryUsage().rss,
          heapUsed: process.memoryUsage().heapUsed,
          maxRssKiB: usage.maxRSS,
          userCpuMs: usage.userCPUTime / 1000,
          systemCpuMs: usage.systemCPUTime / 1000,
          eventLoopP95Ms: loopDelay.percentile(95) / 1e6,
          eventLoopMaxMs: loopDelay.max / 1e6,
        },
      }),
    )
  } finally {
    loopDelay.disable()
    if (core) await core.close()
    await rm(dataDir, { recursive: true, force: true })
  }
}

async function runWorker(implementation, pair, order) {
  const child = spawn(
    process.execPath,
    [fileURLToPath(import.meta.url), '--worker', implementation],
    {
      cwd: projectRoot,
      env: process.env,
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
      `${implementation} puzzle worker failed in pair ${pair}: ${status.signal ?? status.code}\n${stderr}\n${stdout}`,
    )
  const resultLine = stdout
    .trim()
    .split('\n')
    .reverse()
    .find((line) => line.trim().startsWith('{'))
  if (!resultLine)
    throw new Error(`Missing ${implementation} puzzle-worker JSON: ${stderr}\n${stdout}`)
  return { pair, order, ...JSON.parse(resultLine), stderr: stderr.trim() || undefined }
}

if (process.argv[2] === '--worker') {
  await worker(process.argv[3])
} else {
  const runs = []
  for (let pair = 1; pair <= pairs; pair++) {
    const order = pair % 2 ? ['typescript', 'rust'] : ['rust', 'typescript']
    for (const implementation of order) {
      console.error(`Puzzle pair ${pair}/${pairs}: ${implementation}`)
      runs.push(await runWorker(implementation, pair, order.join(' then ')))
    }
  }
  const metrics = [...new Set(runs.flatMap((run) => Object.keys(run.samples)))].sort()
  const percentile = (values, fraction) => {
    const sorted = [...values].sort((a, b) => a - b)
    return sorted[Math.min(sorted.length - 1, Math.ceil(fraction * sorted.length) - 1)] ?? null
  }
  const aggregate = Object.fromEntries(
    metrics.map((metric) => [
      metric,
      Object.fromEntries(
        ['typescript', 'rust'].map((implementation) => {
          const values = runs
            .filter((run) => run.implementation === implementation)
            .flatMap((run) => run.samples[metric] ?? [])
          return [
            implementation,
            { n: values.length, p50Ms: percentile(values, 0.5), p95Ms: percentile(values, 0.95) },
          ]
        }),
      ),
    ]),
  )
  const result = {
    schemaVersion: 1,
    title: 'KChess offline puzzle database query benchmark',
    capturedAt: new Date().toISOString(),
    revisions: { rust: '8f58da7', typescript: 'f479f7c' },
    environment: {
      hostname: hostname(),
      platform: `${platform()} ${release()}`,
      arch: arch(),
      node: process.version,
      cpu: cpus()[0]?.model,
      logicalCpuCount: cpus().length,
      totalMemoryBytes: totalmem(),
    },
    methodology: {
      pairs,
      order: 'Alternates TypeScript-first and Rust-first independent worker processes.',
      fixture:
        '10k and 100k deterministic puzzle rows with a valid e2e4/e7e5 two-ply line, seeded transactionally into the service-created puzzles.db schema. Identical IDs, ratings, theme distribution, FEN and solution rows in each version.',
      operations:
        'Measures public puzzleDbStatus, localPuzzles with mateIn2/rating filters, and localLadder. Twenty query samples per API and fixture size per worker.',
      import:
        'Does not benchmark network download or puzzleDbInstall; fixture seeding is direct SQLite setup, excluded from API latency and recorded separately.',
      output:
        'Random query IDs are intentionally not compared; every result is checked for count, rating range, exact theme token, and valid returned solution line.',
      eventLoop:
        'perf_hooks delay histogram spans database fixture setup and API calls; per-call query latency arrays isolate API costs. Do not interpret the whole-worker event-loop maximum as one user action.',
    },
    aggregate,
    runs,
  }
  await mkdir(join(projectRoot, 'docs/benchmarks/raw'), { recursive: true })
  await writeFile(outputPath, `${JSON.stringify(result, null, 2)}\n`)
  console.log(`Wrote ${outputPath}`)
}
