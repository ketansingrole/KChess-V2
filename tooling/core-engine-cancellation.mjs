import { spawn } from 'node:child_process'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import { arch, cpus, hostname, platform, release, totalmem } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { performance } from 'node:perf_hooks'
import { attemptAsync } from './capture-error.mjs'

const projectRoot = fileURLToPath(new URL('..', import.meta.url))
const baselineRoot = resolve(
  process.env.KCHESS_BENCHMARK_BASELINE_ROOT ?? '/private/tmp/kchess-benchmark-ts',
)
const baselineEntry = join(baselineRoot, 'core/dist/index.js')
const outputPath = resolve(
  process.env.KCHESS_ENGINE_CANCELLATION_OUTPUT ??
    join(projectRoot, 'docs/benchmarks/raw/core-engine-cancellation-2026-10.json'),
)

async function worker(implementation) {
  const temp = await import('node:os').then(({ tmpdir }) => tmpdir())
  const dataDir = join(temp, `kchess-engine-cancel-${process.pid}-${implementation}`)
  await mkdir(dataDir, { recursive: true })
  await writeFile(join(dataDir, 'kchess-data.json'), '{}\n')
  await writeFile(join(dataDir, 'lichess-tokens.json'), '{}\n')
  let core
  try {
    const importStart = performance.now()
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
    const importMs = performance.now() - importStart
    core = createKChessCore({
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
    })
    await core.loadData()
    await core.resumeOnline()
    const status = await core.engineStatus()
    if (!status.ready || !status.bundled) throw new Error('Bundled engine is not ready')

    const fixedStart = performance.now()
    const move = await core.bestMove(['e2e4'], 'max', { movetime: 160 })
    const fixedMs = performance.now() - fixedStart
    const moveValid = /^[a-h][1-8][a-h][1-8][qrbn]?$/.test(move)

    let stopRequested = false
    let stopError
    let cancellationError
    const cancelStart = performance.now()
    const pending = core.bestMove(['e2e4'], 'max', { movetime: 5000 })
    let timer
    const stopCalled = new Promise((resolveStop) => {
      timer = setTimeout(async () => {
        stopRequested = true
        const outcome = await attemptAsync(() => core.stopEngine())
        if ('error' in outcome) stopError = outcome.error
        resolveStop()
      }, 250)
    })
    let resolved = false
    try {
      await pending
      resolved = true
    } catch (error) {
      console.warn(`engine cancellation probe rejected: ${error?.message ?? error}`)
      cancellationError = { name: error?.name ?? 'Error', message: String(error?.message ?? error) }
    }
    const cancellationMs = performance.now() - cancelStart
    clearTimeout(timer)
    if (!stopRequested) await stopCalled

    console.log(
      JSON.stringify({
        implementation,
        importMs,
        fixedMs,
        moveValid,
        cancellationMs,
        stopRequested,
        resolved,
        cancellationError,
        stopError,
      }),
    )
  } finally {
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
      env: { ...process.env },
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
      `${implementation} engine worker failed in pair ${pair}: ${status.signal ?? status.code}\n${stderr}\n${stdout}`,
    )
  const resultLine = stdout
    .trim()
    .split('\n')
    .reverse()
    .find((line) => line.trim().startsWith('{'))
  if (!resultLine)
    throw new Error(`Missing JSON from ${implementation} engine worker: ${stderr}\n${stdout}`)
  const result = JSON.parse(resultLine)
  return { pair, order, ...result, stderr: stderr.trim() || undefined }
}

if (process.argv[2] === '--worker') {
  await worker(process.argv[3])
} else {
  const runs = []
  for (let pair = 1; pair <= 7; pair++) {
    const order = pair % 2 === 1 ? ['typescript', 'rust'] : ['rust', 'typescript']
    for (const implementation of order) {
      console.error(`Engine pair ${pair}/7: ${implementation}`)
      runs.push(await runWorker(implementation, pair, order.join(' then ')))
    }
  }
  const parity = Array.from({ length: 7 }, (_, index) => {
    const pair = index + 1
    const ts = runs.find((run) => run.pair === pair && run.implementation === 'typescript')
    const rust = runs.find((run) => run.pair === pair && run.implementation === 'rust')
    return {
      pair,
      moveValidityMatches: ts.moveValid === rust.moveValid,
      bothIssuedStop: ts.stopRequested && rust.stopRequested,
    }
  })
  const result = {
    schemaVersion: 1,
    title: 'KChess bundled Stockfish cancellation follow-up',
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
      pairs: 7,
      order: 'Alternates TypeScript-first and Rust-first across independent worker processes.',
      calls:
        'Both revisions use their bundled Stockfish, max level, Threads=4 on this 10-logical-CPU host, Hash=64, fixed movetime 160 ms; cancellation starts a max-level search at the API maximum of 5000 ms and calls stopEngine after 250 ms.',
      cancellation:
        'Records whether stopEngine was requested, whether the search promise resolved or rejected, elapsed time and error. No puzzle database or large game fixture is opened; each worker uses a fresh temp profile.',
      caveats: [
        'Timed searches can vary with OS scheduling and engine process startup.',
        'Cancellation elapsed time includes dispatch and stop propagation; stopRequested confirms the timer fired, while the error records API behavior.',
      ],
    },
    parity,
    runs,
  }
  await mkdir(join(projectRoot, 'docs/benchmarks/raw'), { recursive: true })
  await writeFile(outputPath, `${JSON.stringify(result, null, 2)}\n`)
  console.log(`Wrote ${outputPath}`)
  console.log(`Stop requested in ${parity.filter((row) => row.bothIssuedStop).length}/7 pairs`)
}
