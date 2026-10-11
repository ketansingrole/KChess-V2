import { DatabaseSync } from 'node:sqlite'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { monitorEventLoopDelay, performance } from 'node:perf_hooks'

const [implementation, projectRoot, profileBase, ...sizeArgs] = process.argv.slice(2)
const sizes = sizeArgs.map(Number)
const root = resolve(projectRoot)
const profileRoot = resolve(profileBase)
const started = performance.now()
const memory = {
  start: process.memoryUsage(),
  peakRss: process.memoryUsage().rss,
  peakHeapUsed: 0,
  peakExternal: 0,
}
let memoryTimer
const loopDelay = monitorEventLoopDelay({ resolution: 10 })
let maxEventLoopGapMs = 0
let lastTick = performance.now()
let tickTimer
const samples = {}
const signatures = {}
const profilePhases = {}
let core
let host
let dataDir
let createKChessCore
let bundledEnginePath

function sample(name, value) {
  ;(samples[name] ??= []).push(value)
}

async function measure(name, work) {
  const before = performance.now()
  const value = await work()
  sample(name, performance.now() - before)
  return value
}

function makeHost(dir) {
  return {
    dataDir: dir,
    bundledEnginePath,
    managedEngineDir: join(dir, 'engines'),
    secrets: {
      available: () => false,
      encrypt: () => {
        throw new Error('Benchmark secret storage is unavailable')
      },
      decrypt: () => {
        throw new Error('Benchmark secret storage is unavailable')
      },
    },
    openExternal: async () => {},
    onBattery: () => false,
  }
}

async function openCore(dir) {
  await mkdir(dir, { recursive: true })
  await writeFile(join(dir, 'kchess-data.json'), '{}\n')
  await writeFile(join(dir, 'lichess-tokens.json'), '{}\n')
  host = makeHost(dir)
  core = createKChessCore(host)
  await core.loadData()
}

function seedGames(dir, count) {
  const db = new DatabaseSync(join(dir, 'kchess.db'))
  try {
    const account = 'benchmark'
    db.prepare(
      'INSERT OR REPLACE INTO accounts (username, connected, lastSyncedAt) VALUES (?, 0, NULL)',
    ).run(account)
    const insert = db.prepare(`INSERT INTO games
      (account, id, createdAt, lastMoveAt, rated, speed, perf, status, winner, color,
       opponent, opponentRating, playerRating, ratingDiff, opening, moves, pgn)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`)
    const transaction = db.exec('BEGIN IMMEDIATE')
    try {
      for (let index = 0; index < count; index++) {
        const date = 1_700_000_000_000 + index * 60_000
        const outcome = index % 3
        insert.run(
          account,
          `bench-${String(index).padStart(6, '0')}`,
          date,
          date + 180_000,
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
          index % 2 === 0 ? 'Italian Game' : "Queen's Pawn Game",
          'e2e4 e7e5 g1f3 b8c6 f1c4',
          `[Event "Benchmark"]\n[Site "local"]\n[Date "2026.01.01"]\n[White "Player"]\n[Black "Opponent"]\n[Result "*"]\n\n1. e4 e5 2. Nf3 Nc6 3. Bc4 *`,
        )
      }
      db.exec('COMMIT')
    } catch (error) {
      db.exec('ROLLBACK')
      throw error
    }
    void transaction
  } finally {
    db.close()
  }
}

function normalize(value) {
  if (Array.isArray(value)) return value.map(normalize)
  if (!value || typeof value !== 'object') return value
  const result = {}
  for (const key of Object.keys(value).sort()) {
    if (['id', 'updatedAt', 'playedAt', 'createdAt', 'lastMoveAt'].includes(key)) continue
    result[key] = normalize(value[key])
  }
  return result
}

try {
  memoryTimer = setInterval(() => {
    const state = process.memoryUsage()
    memory.peakRss = Math.max(memory.peakRss, state.rss)
    memory.peakHeapUsed = Math.max(memory.peakHeapUsed, state.heapUsed)
    memory.peakExternal = Math.max(memory.peakExternal, state.external)
  }, 10)
  tickTimer = setInterval(() => {
    const now = performance.now()
    maxEventLoopGapMs = Math.max(maxEventLoopGapMs, now - lastTick - 10)
    lastTick = now
  }, 10)
  loopDelay.enable()

  const importStart = performance.now()
  const moduleUrl = pathToFileURL(join(root, 'crates/kchess-node/dist/index.js')).href
  const baselineEntry = process.env.KCHESS_BENCHMARK_ENTRY
  if (implementation === 'typescript') {
    if (!baselineEntry)
      throw new Error('KCHESS_BENCHMARK_ENTRY is required for the TypeScript baseline')
    const baseline = await import(pathToFileURL(resolve(baselineEntry)).href)
    createKChessCore = baseline.createKChessCore
    bundledEnginePath = join(
      resolve(process.env.KCHESS_BENCHMARK_BASELINE_ROOT),
      baseline.BUNDLED_ENGINE_SCRIPT,
    )
  } else {
    const current = await import(moduleUrl)
    createKChessCore = current.createKChessCore
    bundledEnginePath = join(root, current.BUNDLED_ENGINE_SCRIPT)
  }
  const importMs = performance.now() - importStart

  // Empty-profile startup includes constructing the API facade, migrations and the first data read.
  dataDir = join(profileRoot, 'startup')
  await mkdir(dataDir, { recursive: true })
  await writeFile(join(dataDir, 'kchess-data.json'), '{}\n')
  await writeFile(join(dataDir, 'lichess-tokens.json'), '{}\n')
  const createStart = performance.now()
  host = makeHost(dataDir)
  core = createKChessCore(host)
  const initialData = await core.loadData()
  const startupMs = performance.now() - createStart
  signatures.startup = {
    accountCount: initialData.accounts.length,
    settingsKeys: Object.keys(initialData.settings).sort(),
  }
  await core.close()
  core = undefined

  for (const count of sizes) {
    dataDir = join(profileRoot, `games-${count}`)
    await mkdir(dataDir, { recursive: true })
    await openCore(dataDir)
    await core.close()
    core = undefined
    seedGames(dataDir, count)
    await openCore(dataDir)

    const pageQuery = { account: 'benchmark', offset: 0, limit: 100 }
    const insightQuery = { account: 'benchmark', days: 3650 }
    const page = await measure(`games_${count}_gamePage_cold`, () => core.gamePage(pageQuery))
    const overview = await measure(`games_${count}_gameLibraryOverview_cold`, () =>
      core.gameLibraryOverview(),
    )
    const insights = await measure(`games_${count}_insights_cold`, () =>
      core.insights(insightQuery),
    )
    const history = await measure(`games_${count}_libraryHistory_cold`, () => core.library())
    for (let i = 0; i < 12; i++) {
      await measure(`games_${count}_gamePage_warm`, () =>
        core.gamePage({ ...pageQuery, offset: (i % 5) * 100 }),
      )
      await measure(`games_${count}_gameLibraryOverview_warm`, () => core.gameLibraryOverview())
      await measure(`games_${count}_insights_warm`, () => core.insights(insightQuery))
      await measure(`games_${count}_libraryHistory_warm`, () => core.library())
    }
    signatures[`games_${count}`] = {
      total: page.total,
      pageLength: page.games.length,
      pageFirst: page.games[0]?.id,
      overview: normalize(overview),
      insights: normalize(insights),
      history: normalize(history),
    }

    const workloadCpuStart = process.resourceUsage()
    const workloadMemoryStart = process.memoryUsage()
    const workloadStart = performance.now()

    const loaded = await core.loadData()
    const initialSettings = loaded.settings
    for (let i = 0; i < 10; i++) {
      await measure(`games_${count}_saveSettings`, () =>
        core.saveSettings({ ...initialSettings, cloudEval: i % 2 === 0 }),
      )
    }
    const study = await core.studyCommand({
      op: 'save',
      name: 'Benchmark Study',
      pgn: '1. e4 e5 *',
    })
    const studyId = study.id
    if (!studyId) throw new Error('studyCommand did not return a study ID')
    for (let i = 0; i < 10; i++) {
      await measure(`games_${count}_studySavePgn`, () =>
        core.studyCommand({
          op: 'save',
          id: studyId,
          name: 'Benchmark Study',
          pgn: i % 2 ? '1. d4 d5 *' : '1. e4 e5 *',
        }),
      )
      await measure(`games_${count}_sessionPersistence`, () =>
        core.saveSession('analysis', {
          pgn: i % 2 ? '1. d4 d5 *' : '1. e4 e5 *',
          path: i % 2 ? 'd2d4' : 'e2e4',
          orientation: 'white',
          study: studyId,
          chapter: '',
        }),
      )
      await measure(`games_${count}_trainingRun`, () =>
        core.saveRun({
          kind: 'storm',
          variant: '3min',
          score: i,
          detail: { elapsedMs: 1000, correct: i },
        }),
      )
    }
    signatures[`writes_${count}`] = {
      studyCount: (await core.library()).studies.length,
      session: normalize((await core.library()).sessions.analysis),
      runs: normalize(await core.runSummary('storm')),
    }
    const workloadCpuEnd = process.resourceUsage()
    const workloadMemoryEnd = process.memoryUsage()
    profilePhases[`games_${count}`] = {
      workloadElapsedMs: performance.now() - workloadStart,
      workloadUserCpuMs: (workloadCpuEnd.userCPUTime - workloadCpuStart.userCPUTime) / 1000,
      workloadSystemCpuMs: (workloadCpuEnd.systemCPUTime - workloadCpuStart.systemCPUTime) / 1000,
      workloadStartRss: workloadMemoryStart.rss,
      workloadEndRss: workloadMemoryEnd.rss,
      workloadStartHeapUsed: workloadMemoryStart.heapUsed,
      workloadEndHeapUsed: workloadMemoryEnd.heapUsed,
      workloadStartExternal: workloadMemoryStart.external,
      workloadEndExternal: workloadMemoryEnd.external,
    }
    await core.close()
    core = undefined
  }

  // Engine calls use the same bundled Stockfish asset; one fixed 160 ms search and one stop request.
  dataDir = join(profileRoot, 'engine')
  await openCore(dataDir)
  await core.resumeOnline()
  signatures.engine = { ready: (await core.engineStatus()).ready }
  const move = await measure('engine_bestMove_160ms', () =>
    core.bestMove(['e2e4'], 'max', { movetime: 160 }),
  )
  signatures.engine.moveValid = /^[a-h][1-8][a-h][1-8][qrbn]?$/.test(move)
  const cancelStart = performance.now()
  const engineCore = core
  const pending = engineCore.bestMove(['e2e4'], 'max', { movetime: 5000 })
  let stopRequested = false
  const stopTimer = setTimeout(() => {
    stopRequested = true
    void engineCore.stopEngine().catch((error) => {
      signatures.engine.stopError = String(error?.message ?? error)
    })
  }, 250)
  try {
    await pending
    sample('engine_cancel_250ms', performance.now() - cancelStart)
    signatures.engine.cancelled = false
  } catch (error) {
    sample('engine_cancel_250ms', performance.now() - cancelStart)
    signatures.engine.cancelled =
      error?.name === 'AbortError' || /cancel|stop/i.test(String(error?.message))
  } finally {
    clearTimeout(stopTimer)
  }
  signatures.engine.stopRequested = stopRequested
  await core.close()
  core = undefined

  const resourceUsage = process.resourceUsage()
  const endMemory = process.memoryUsage()
  const normalizedSamples = Object.fromEntries(
    Object.entries(samples).map(([key, values]) => [key, values]),
  )
  console.log(
    JSON.stringify({
      implementation,
      sizes,
      importMs,
      startupMs,
      elapsedMs: performance.now() - started,
      samples: normalizedSamples,
      signatures,
      profilePhases,
      memory: {
        startRss: memory.start.rss,
        endRss: endMemory.rss,
        peakRss: memory.peakRss,
        startHeapUsed: memory.start.heapUsed,
        endHeapUsed: endMemory.heapUsed,
        peakHeapUsed: memory.peakHeapUsed,
        startExternal: memory.start.external,
        endExternal: endMemory.external,
        peakExternal: memory.peakExternal,
        maxRssKiB: resourceUsage.maxRSS,
        userCpuMs: resourceUsage.userCPUTime / 1000,
        systemCpuMs: resourceUsage.systemCPUTime / 1000,
        maxEventLoopGapMs,
        eventLoopP95Ms: loopDelay.percentile(95) / 1e6,
      },
    }),
  )
} finally {
  clearInterval(memoryTimer)
  clearInterval(tickTimer)
  loopDelay.disable()
  if (core) await core.close()
  await rm(profileRoot, { recursive: true, force: true })
}
