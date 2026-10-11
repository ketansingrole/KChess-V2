#!/usr/bin/env node
// Paired, deterministic loopback measurements of public Lichess CoreApi calls.
import { createServer } from 'node:http'
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const baselineRoot = '/private/tmp/kchess-benchmark-ts'
const rawPath =
  process.env.KCHESS_NETWORK_OUTPUT ?? path.join(root, 'docs/benchmarks/raw/network-services.json')
const isWorker = process.argv[2] === '--worker'
const isBurstWorker = process.argv[2] === '--burst-worker'
const workerVariant = process.argv[3]
const workerBase = process.argv[4]
const workerProfile = process.argv[5]
const baselineRevision = 'f479f7c'
const currentRevision = process.env.KCHESS_BENCHMARK_REVISION ?? '8f58da7'
const repetitions = 4
const methods = ['profile', 'recentGames', 'tvChannels']
const burstMethods = ['sameKey', 'distinctKeys']

const game = {
  id: 'Bench001',
  createdAt: 1_700_000_000_000,
  lastMoveAt: 1_700_000_001_000,
  rated: true,
  speed: 'blitz',
  perf: 'blitz',
  status: 'mate',
  winner: 'white',
  players: {
    white: { user: { name: 'FixtureUser' }, rating: 1800, ratingDiff: 5 },
    black: { user: { name: 'FixtureOpponent' }, rating: 1750, ratingDiff: -5 },
  },
  opening: { name: 'Sicilian Defense' },
  moves: 'e4 c5',
}
const fixturePayloads = {
  profile: {
    id: 'fixtureuser',
    username: 'FixtureUser',
    title: 'GM',
    online: false,
    perfs: { blitz: { games: 100, rating: 2000, rd: 50, prog: 5 } },
    createdAt: 1_600_000_000_000,
    seenAt: 1_700_000_000_000,
  },
  tvChannels: {
    best: { gameId: 'TVBest01', user: { name: 'TopFixture', title: 'GM' }, rating: 2600 },
    blitz: { gameId: 'TVBlitz1', user: { name: 'BlitzFixture' }, rating: 2200 },
    rapid: { gameId: 'TVRapid1', user: { name: 'RapidFixture' }, rating: 2100 },
  },
}

function sha256(value) {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex')
}

function normalizeResult(value) {
  if (Array.isArray(value)) return value.map((item) => normalizeResult(item))
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value)
        .filter(([, item]) => item !== undefined)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, item]) => [key, normalizeResult(item)]),
    )
  }
  return value
}

function median(values) {
  const sorted = [...values].sort((a, b) => a - b)
  if (!sorted.length) return null
  const middle = Math.floor(sorted.length / 2)
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2
}

async function worker() {
  if (!['rust', 'typescript'].includes(workerVariant) || !workerBase || !workerProfile)
    throw new Error('Invalid network worker arguments.')
  const realFetch = globalThis.fetch.bind(globalThis)
  if (workerVariant === 'typescript') {
    // The baseline client hardcodes https://lichess.org; intercept before importing its bundle.
    globalThis.fetch = async (input, init) => {
      const request = input instanceof Request ? input : new Request(input, init)
      const originalUrl = new URL(request.url)
      const fixtureUrl = new URL(originalUrl.pathname + originalUrl.search, workerBase)
      return realFetch(new Request(fixtureUrl, request))
    }
  }

  let core
  const measurements = []
  try {
    let module
    if (workerVariant === 'rust') {
      module = await import(pathToFileURL(path.join(root, 'crates/kchess-node/dist/index.js')))
    } else {
      module = await import(pathToFileURL(path.join(baselineRoot, 'core/dist/index.js')))
    }
    const bundledEnginePath = path.join(
      workerVariant === 'rust' ? root : baselineRoot,
      module.BUNDLED_ENGINE_SCRIPT,
    )
    const host = {
      dataDir: workerProfile,
      bundledEnginePath,
      managedEngineDir: path.join(workerProfile, 'engines'),
      secrets: {
        available: () => false,
        encrypt: () => {
          throw new Error('No secrets in the benchmark host.')
        },
        decrypt: () => {
          throw new Error('No secrets in the benchmark host.')
        },
      },
      openExternal: async () => {},
      onBattery: () => false,
    }
    core = module.createKChessCore(host)
    // Initialize the same disposable store before timing any Lichess request.
    await core.loadData()
    const invoke = {
      profile: () => core.profile('FixtureUser'),
      recentGames: () => core.recentGames('FixtureUser'),
      tvChannels: () => core.tvChannels(),
    }
    for (const method of methods) {
      for (let iteration = 0; iteration <= repetitions; iteration += 1) {
        const phase = iteration === 0 ? 'cold' : 'warm'
        await realFetch(
          `${workerBase}/__bench/mark?phase=start&method=${method}&iteration=${iteration}`,
        ).then((response) => response.arrayBuffer())
        const started = performance.now()
        const value = await invoke[method]()
        const durationMs = performance.now() - started
        const normalizedResult = normalizeResult(value)
        const marker = await realFetch(
          `${workerBase}/__bench/mark?phase=end&method=${method}&iteration=${iteration}`,
        ).then((response) => response.json())
        measurements.push({
          method,
          phase,
          iteration,
          durationMs: Number(durationMs.toFixed(3)),
          requestCounts: marker.requestCounts,
          resultCount: Array.isArray(value) ? value.length : undefined,
          resultSha256: sha256(normalizedResult),
          ...(iteration === 0 ? { normalizedResult } : {}),
        })
      }
    }
    process.stdout.write(
      `NETWORK_WORKER_RESULT=${JSON.stringify({
        variant: workerVariant,
        revision: workerVariant === 'rust' ? currentRevision : baselineRevision,
        measurements,
      })}\n`,
    )
  } finally {
    if (core) await core.close()
  }
}

async function burstWorker() {
  if (!['rust', 'typescript'].includes(workerVariant) || !workerBase || !workerProfile)
    throw new Error('Invalid concurrent-profile worker arguments.')
  const realFetch = globalThis.fetch.bind(globalThis)
  if (workerVariant === 'typescript') {
    // The baseline client hardcodes origin; intercept only its outgoing requests before import.
    globalThis.fetch = async (input, init) => {
      const request = input instanceof Request ? input : new Request(input, init)
      const originalUrl = new URL(request.url)
      const fixtureUrl = new URL(originalUrl.pathname + originalUrl.search, workerBase)
      return realFetch(new Request(fixtureUrl, request))
    }
  }
  let core
  const bursts = []
  try {
    const module =
      workerVariant === 'rust'
        ? await import(pathToFileURL(path.join(root, 'crates/kchess-node/dist/index.js')))
        : await import(pathToFileURL(path.join(baselineRoot, 'core/dist/index.js')))
    const bundledEnginePath = path.join(
      workerVariant === 'rust' ? root : baselineRoot,
      module.BUNDLED_ENGINE_SCRIPT,
    )
    core = module.createKChessCore({
      dataDir: workerProfile,
      bundledEnginePath,
      managedEngineDir: path.join(workerProfile, 'engines'),
      secrets: {
        available: () => false,
        encrypt: () => {
          throw new Error('No secrets in the benchmark host.')
        },
        decrypt: () => {
          throw new Error('No secrets in the benchmark host.')
        },
      },
      openExternal: async () => {},
      onBattery: () => false,
    })
    await core.loadData()
    const callCount = 8
    for (const kind of burstMethods) {
      const usernames =
        kind === 'sameKey'
          ? Array.from({ length: callCount }, () => 'FixtureUser')
          : Array.from({ length: callCount }, (_, index) => `FixtureUser${index + 1}`)
      await realFetch(`${workerBase}/__bench/mark?phase=start&burst=${kind}`).then((response) =>
        response.arrayBuffer(),
      )
      const started = performance.now()
      const values = await Promise.all(usernames.map((username) => core.profile(username)))
      const durationMs = performance.now() - started
      const marker = await realFetch(`${workerBase}/__bench/mark?phase=end&burst=${kind}`).then(
        (response) => response.json(),
      )
      const normalizedResults = values.map((value) => normalizeResult(value))
      bursts.push({
        kind,
        requestedCalls: callCount,
        durationMs: Number(durationMs.toFixed(3)),
        requestCounts: marker.requestCounts,
        resultCount: values.length,
        allResultsSha256: sha256(normalizedResults),
        normalizedResults,
      })
    }
    process.stdout.write(
      `NETWORK_BURST_RESULT=${JSON.stringify({
        variant: workerVariant,
        revision: workerVariant === 'rust' ? currentRevision : baselineRevision,
        bursts,
      })}\n`,
    )
  } finally {
    if (core) await core.close()
  }
}

async function main() {
  if (isBurstWorker) return burstWorker()
  if (isWorker) return worker()
  const inflightMode = process.argv.includes('--inflight')
  const outPath =
    process.env.KCHESS_NETWORK_OUTPUT ??
    (inflightMode ? path.join(root, 'docs/benchmarks/raw/network-inflight.json') : rawPath)
  const events = []
  let markerCursor = 0
  const server = createServer(async (request, response) => {
    const url = new URL(request.url ?? '/', 'http://127.0.0.1')
    if (url.pathname === '/__bench/mark') {
      const phase = url.searchParams.get('phase')
      if (phase === 'start') markerCursor = events.length
      const counts = {}
      for (const item of events.slice(markerCursor))
        counts[item.path] = (counts[item.path] ?? 0) + 1
      markerCursor = events.length
      response.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' })
      response.end(JSON.stringify({ requestCounts: counts }))
      return
    }
    events.push({ path: url.pathname, at: new Date().toISOString() })
    await new Promise((resolve) => setTimeout(resolve, inflightMode ? 100 : 2))
    if (url.pathname.startsWith('/api/user/')) {
      const username = decodeURIComponent(url.pathname.slice('/api/user/'.length))
      const profile = { ...fixturePayloads.profile, id: username.toLowerCase(), username }
      response.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' })
      response.end(JSON.stringify(profile))
    } else if (url.pathname === '/api/games/user/FixtureUser') {
      response.writeHead(200, {
        'content-type': 'application/x-ndjson',
        'cache-control': 'no-store',
      })
      response.end(`${JSON.stringify(game)}\n`)
    } else if (url.pathname === '/api/tv/channels') {
      response.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' })
      response.end(JSON.stringify(fixturePayloads.tvChannels))
    } else {
      response.writeHead(404, { 'content-type': 'application/json' })
      response.end('{"error":"fixture route not found"}')
    }
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  const fixtureBase = `http://127.0.0.1:${address.port}`
  const raw = {
    schemaVersion: 1,
    study: inflightMode ? 'in-flight profile request coalescing' : 'serial Lichess service calls',
    startedAt: new Date().toISOString(),
    host: {
      platform: process.platform,
      release: os.release(),
      architecture: os.arch(),
      cpuModel: os.cpus()[0]?.model ?? null,
      logicalCpuCount: os.cpus().length,
      nodeVersion: process.version,
    },
    methodology: {
      variants:
        'Rust @ 8f58da7 vs extracted TypeScript baseline @ f479f7c; calls made through each revision’s actual createKChessCore CoreApi.',
      fixture: inflightMode
        ? 'Loopback HTTP only: deterministic profile JSON for same-key and distinct-key concurrency bursts. No account, token, or external network request is used. Each profile response waits 100 ms to expose duplicate in-flight requests.'
        : 'Loopback HTTP only: deterministic profile JSON, one valid recent-games NDJSON record, and deterministic TV channels JSON. No account, token, or external network request is used. A 2 ms server delay is applied to API responses.',
      cache: inflightMode
        ? 'Fresh temporary data directory per worker process. Initialize via loadData(), then issue one eight-call concurrent same-key burst followed by one eight-call concurrent distinct-key burst. No earlier profile call populates the cache; fixture markers count actual wire requests for each burst.'
        : 'Fresh temporary data directory per worker process. Initialize via loadData(), then one cold call plus four warm repetitions per method. Each measured call is bracketed by fixture markers; raw path request counts show whether cache reduced traffic.',
      order:
        'Three paired worker-process runs; Rust and TypeScript order alternates by pair. Processes run serially.',
      limits: inflightMode
        ? 'This measures request coalescing and same/different-key fanout against a deterministic 100 ms loopback fixture. It does not model real Lichess latency, server rate limiting, or real-world service load. Distinct-key traffic is the control for whether the client can fan out independent requests.'
        : 'Loopback fixture throughput measures client/core overhead, JSON/NDJSON parsing, and cache behavior; it does not estimate real Lichess network latency, rate limiting, or service load.',
    },
    fixtureRequests: [],
    runs: [],
  }

  async function runWorker(variant, pair) {
    const profile = await mkdtemp(path.join(os.tmpdir(), `kchess-network-${variant}-`))
    try {
      const env = {
        ...process.env,
        KCHESS_TEST_LICHESS_BASE: variant === 'rust' ? fixtureBase : '',
        KCHESS_STORE_DEBUG: variant === 'rust' ? '1' : '',
      }
      const child = spawn(
        process.execPath,
        [fileURLToPath(import.meta.url), '--worker', variant, fixtureBase, profile],
        { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] },
      )
      let stdout = ''
      let stderr = ''
      child.stdout.setEncoding('utf8').on('data', (chunk) => {
        stdout += chunk
      })
      child.stderr.setEncoding('utf8').on('data', (chunk) => {
        stderr += chunk
      })
      const exitCode = await new Promise((resolve, reject) => {
        child.once('error', reject)
        child.once('exit', resolve)
      })
      const resultLine = stdout
        .split('\n')
        .find((line) => line.startsWith('NETWORK_WORKER_RESULT='))
      if (exitCode !== 0 || !resultLine) {
        throw new Error(`${variant} worker failed with exit ${exitCode}: ${stderr}\n${stdout}`)
      }
      const result = JSON.parse(resultLine.slice('NETWORK_WORKER_RESULT='.length))
      return { pair, ...result }
    } finally {
      await rm(profile, { recursive: true, force: true })
    }
  }

  async function runBurstWorker(variant, pair) {
    const profile = await mkdtemp(path.join(os.tmpdir(), `kchess-network-burst-${variant}-`))
    try {
      const env = {
        ...process.env,
        KCHESS_TEST_LICHESS_BASE: variant === 'rust' ? fixtureBase : '',
        KCHESS_STORE_DEBUG: variant === 'rust' ? '1' : '',
      }
      const child = spawn(
        process.execPath,
        [fileURLToPath(import.meta.url), '--burst-worker', variant, fixtureBase, profile],
        { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] },
      )
      let stdout = ''
      let stderr = ''
      child.stdout.setEncoding('utf8').on('data', (chunk) => {
        stdout += chunk
      })
      child.stderr.setEncoding('utf8').on('data', (chunk) => {
        stderr += chunk
      })
      const exitCode = await new Promise((resolve, reject) => {
        child.once('error', reject)
        child.once('exit', resolve)
      })
      const resultLine = stdout.split('\n').find((line) => line.startsWith('NETWORK_BURST_RESULT='))
      if (exitCode !== 0 || !resultLine)
        throw new Error(
          `${variant} burst worker failed with exit ${exitCode}: ${stderr}\n${stdout}`,
        )
      return { pair, ...JSON.parse(resultLine.slice('NETWORK_BURST_RESULT='.length)) }
    } finally {
      await rm(profile, { recursive: true, force: true })
    }
  }

  try {
    for (let pair = 1; pair <= 3; pair += 1) {
      const order = pair % 2 ? ['rust', 'typescript'] : ['typescript', 'rust']
      for (const variant of order) {
        raw.runs.push(
          inflightMode ? await runBurstWorker(variant, pair) : await runWorker(variant, pair),
        )
      }
    }
    raw.fixtureRequests = events
    if (inflightMode) {
      raw.requestParity = Object.fromEntries(
        burstMethods.map((kind) => {
          const requests = Object.fromEntries(
            ['rust', 'typescript'].map((variant) => [
              variant,
              raw.runs
                .filter((run) => run.variant === variant)
                .flatMap((run) => run.bursts.filter((burst) => burst.kind === kind))
                .map((burst) =>
                  Object.values(burst.requestCounts).reduce((sum, count) => sum + count, 0),
                ),
            ]),
          )
          const outputs = Object.fromEntries(
            ['rust', 'typescript'].map((variant) => [
              variant,
              raw.runs
                .filter((run) => run.variant === variant)
                .flatMap((run) => run.bursts.filter((burst) => burst.kind === kind))
                .map((burst) => burst.allResultsSha256),
            ]),
          )
          return [
            kind,
            {
              requestsByVariant: requests,
              outputHashesByVariant: outputs,
              outputMatched: new Set([...outputs.rust, ...outputs.typescript]).size === 1,
            },
          ]
        }),
      )
      raw.summary = Object.fromEntries(
        ['rust', 'typescript'].map((variant) => [
          variant,
          Object.fromEntries(
            burstMethods.map((kind) => {
              const samples = raw.runs
                .filter((run) => run.variant === variant)
                .flatMap((run) => run.bursts.filter((burst) => burst.kind === kind))
              return [
                kind,
                {
                  samples: samples.length,
                  medianDurationMs: median(samples.map((sample) => sample.durationMs)),
                  medianWireRequests: median(
                    samples.map((sample) =>
                      Object.values(sample.requestCounts).reduce((sum, count) => sum + count, 0),
                    ),
                  ),
                },
              ]
            }),
          ),
        ]),
      )
      raw.completedAt = new Date().toISOString()
      await mkdir(path.dirname(outPath), { recursive: true })
      await writeFile(outPath, `${JSON.stringify(raw, null, 2)}\n`)
      process.stdout.write(`${outPath}\n`)
      return
    }
    raw.resultParity = Object.fromEntries(
      methods.map((method) => {
        const hashes = Object.fromEntries(
          ['rust', 'typescript'].map((variant) => {
            const values = raw.runs
              .filter((run) => run.variant === variant)
              .flatMap((run) => run.measurements.filter((entry) => entry.method === method))
              .map((entry) => entry.resultSha256)
            return [variant, [...new Set(values)]]
          }),
        )
        return [
          method,
          {
            matched:
              hashes.rust.length === 1 &&
              hashes.typescript.length === 1 &&
              hashes.rust[0] === hashes.typescript[0],
            resultSha256ByVariant: hashes,
          },
        ]
      }),
    )
    raw.comparisonEligibility = Object.fromEntries(
      methods.map((method) => [
        method,
        {
          eligible: raw.resultParity[method].matched,
          reason: raw.resultParity[method].matched
            ? 'Normalized outputs match across variants.'
            : 'Outputs differ after object-key canonicalization; timing is retained for audit but must not be interpreted as a performance comparison.',
        },
      ]),
    )
    raw.summary = Object.fromEntries(
      ['rust', 'typescript'].map((variant) => [
        variant,
        Object.fromEntries(
          methods.map((method) => [
            method,
            Object.fromEntries(
              ['cold', 'warm'].map((phase) => {
                const samples = raw.runs
                  .filter((run) => run.variant === variant)
                  .flatMap((run) =>
                    run.measurements.filter(
                      (entry) => entry.method === method && entry.phase === phase,
                    ),
                  )
                const requestCounts = {}
                for (const sample of samples) {
                  for (const [path, count] of Object.entries(sample.requestCounts)) {
                    requestCounts[path] = (requestCounts[path] ?? 0) + count
                  }
                }
                return [
                  phase,
                  {
                    samples: samples.length,
                    medianDurationMs: median(samples.map((sample) => sample.durationMs)),
                    medianRequestsPerCall: median(
                      samples.map((sample) =>
                        Object.values(sample.requestCounts).reduce((sum, n) => sum + n, 0),
                      ),
                    ),
                    totalRequestsByPath: requestCounts,
                  },
                ]
              }),
            ),
          ]),
        ),
      ]),
    )
    raw.completedAt = new Date().toISOString()
    await mkdir(path.dirname(outPath), { recursive: true })
    await writeFile(outPath, `${JSON.stringify(raw, null, 2)}\n`)
    process.stdout.write(`${outPath}\n`)
  } finally {
    server.closeAllConnections()
    await new Promise((resolve) => server.close(resolve))
  }
}

await main()
