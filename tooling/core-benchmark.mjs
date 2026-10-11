import { spawn } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import { arch, cpus, hostname, platform, release, totalmem } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const projectRoot = fileURLToPath(new URL('..', import.meta.url))
const baselineRoot = resolve(
  process.env.KCHESS_BENCHMARK_BASELINE_ROOT ?? '/private/tmp/kchess-benchmark-ts',
)
const baselineEntry = join(baselineRoot, 'core/dist/index.js')
const outputPath = resolve(
  process.env.KCHESS_BENCHMARK_OUTPUT ??
    join(projectRoot, 'docs/benchmarks/raw/core-rust-vs-typescript-2026-10.json'),
)
const args = process.argv.slice(2)
const pairsArg = args.find((arg) => arg.startsWith('--pairs='))
const sizesArg = args.find((arg) => arg.startsWith('--sizes='))
const pairs = Number(pairsArg?.split('=')[1] ?? 7)
const sizes = (sizesArg?.split('=')[1] ?? '1000,5000,50000').split(',').map(Number)
if (!Number.isInteger(pairs) || pairs < 1 || sizes.some((n) => !Number.isInteger(n) || n < 1)) {
  throw new Error('Usage: node tooling/core-benchmark.mjs [--pairs=7] [--sizes=1000,5000,50000]')
}

async function runWorker(implementation, pair, order) {
  const profile = join('/private/tmp', `kchess-core-bench-${process.pid}-${pair}-${implementation}`)
  const worker = join(projectRoot, 'tooling/core-benchmark-worker.mjs')
  const child = spawn(
    process.execPath,
    [worker, implementation, projectRoot, profile, ...sizes.map(String)],
    {
      cwd: projectRoot,
      env: {
        ...process.env,
        KCHESS_BENCHMARK_ENTRY: baselineEntry,
        KCHESS_BENCHMARK_BASELINE_ROOT: baselineRoot,
      },
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
      `${implementation} worker failed in pair ${pair} (order ${order}): ${status.signal ?? status.code}\n${stderr}\n${stdout}`,
    )
  const line = stdout.trim().split('\n').at(-1)
  let result
  try {
    result = JSON.parse(line)
  } catch (error) {
    throw new Error(
      `Could not parse ${implementation} worker output in pair ${pair}: ${error}\n${stderr}\n${stdout}`,
      { cause: error },
    )
  }
  return { pair, order, ...result, stderr: stderr.trim() || undefined }
}

const runs = []
for (let pair = 0; pair < pairs; pair++) {
  const order = pair % 2 === 0 ? ['typescript', 'rust'] : ['rust', 'typescript']
  for (const implementation of order) {
    console.error(`Pair ${pair + 1}/${pairs}: ${implementation} starting`)
    runs.push(await runWorker(implementation, pair + 1, order.join(' then ')))
  }
}

function percentile(values, percentile) {
  if (!values.length) return null
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.ceil(percentile * sorted.length) - 1)]
}

const metrics = [...new Set(runs.flatMap((run) => Object.keys(run.samples)))].sort()
const aggregate = Object.fromEntries(
  metrics.map((metric) => {
    const result = {}
    for (const implementation of ['typescript', 'rust']) {
      const values = runs
        .filter((run) => run.implementation === implementation)
        .flatMap((run) => run.samples[metric] ?? [])
      result[implementation] = {
        n: values.length,
        p50Ms: percentile(values, 0.5),
        p95Ms: percentile(values, 0.95),
        meanMs: values.length
          ? values.reduce((sum, value) => sum + value, 0) / values.length
          : null,
      }
    }
    return [metric, result]
  }),
)

const parity = []
for (let pair = 1; pair <= pairs; pair++) {
  const ts = runs.find((run) => run.pair === pair && run.implementation === 'typescript')
  const rust = runs.find((run) => run.pair === pair && run.implementation === 'rust')
  const parityView = (signatures) => {
    const value = structuredClone(signatures)
    for (const key of Object.keys(value ?? {})) {
      if (key.startsWith('writes_') && value[key]?.session) delete value[key].session.study
    }
    return value
  }
  const same =
    JSON.stringify(parityView(ts?.signatures)) === JSON.stringify(parityView(rust?.signatures))
  parity.push({
    pair,
    same,
    normalization:
      'Ignores the generated study UUID referenced by the saved analysis session; all other signature fields are compared.',
    typescript: ts?.signatures,
    rust: rust?.signatures,
  })
}

const result = {
  schemaVersion: 1,
  title: 'KChess core TypeScript baseline vs Rust native core',
  capturedAt: new Date().toISOString(),
  revisions: {
    rust: process.env.KCHESS_BENCHMARK_REVISION ?? '8f58da7',
    typescript: 'f479f7c',
    rustArtifact: 'crates/kchess-node/dist/index.js built from the checked-out source',
    typescriptArtifact: baselineEntry,
  },
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
    order: 'Alternating TypeScript-first and Rust-first within each independent process pair.',
    sizes,
    gameFixture:
      'Identical deterministic rows seeded via node:sqlite into each implementation’s migrated kchess.db; synthetic local-only 5-ply games, one account, indexed dates and results.',
    measured:
      'Each process dynamically imports its implementation, measures fresh-profile create+loadData, then queries gamePage, gameLibraryOverview, insights, library history, repeats reads, writes settings, saves study PGN, persists analysis sessions, saves training runs, performs bundled Stockfish fixed movetime and an explicit stop/cancel attempt.',
    rawData:
      'Per-operation latency arrays, per-run process resource usage, normalized outputs and paired parity results are retained below.',
    eventLoop:
      'A 10 ms interval records the largest observed gap while the workload runs; short blocks and timer scheduling can be missed. perf_hooks delay p95 is also reported.',
    rss: 'process.resourceUsage().maxRSS is in KiB on macOS; live RSS/heap/external peaks are sampled every 10 ms and can miss brief peaks.',
    cpu: 'process.resourceUsage user/system CPU time for the worker, including fixture seeding; use as process cost rather than API-only CPU.',
    engine:
      'Both use their revision’s bundled Stockfish script at max level with 160 ms movetime; only move validity is compared because time-limited search moves may vary with scheduling. Cancellation uses a valid 5000 ms search stopped after 250 ms. The original historical study used an invalid 10000 ms request; that historical cancellation row must be excluded.',
    caveats: [
      'Synthetic data does not model network sync, varied PGN lengths, account diversity, or real-world query mixes.',
      'Cold process starts alternate, but filesystem and OS caches remain shared.',
      'The TypeScript bundle is the extracted baseline build and its dependencies are from the matching copied node_modules tree.',
      'All measured reads run in one core instance after seeding; fixture construction is excluded from API latency but included in process CPU and elapsed time.',
    ],
  },
  aggregate,
  parity,
  runs,
}

await mkdir(join(projectRoot, 'docs/benchmarks/raw'), { recursive: true })
await writeFile(outputPath, `${JSON.stringify(result, null, 2)}\n`)
console.log(`Wrote ${outputPath}`)
console.log(`Parity: ${parity.filter((pair) => pair.same).length}/${parity.length} pairs match`)
