#!/usr/bin/env node
// Probe tree payload scaling and rule-call counts across the TypeScript/WASM/native paths.
// --verify records output parity and JSON byte/call counts without timing. --measure is an
// explicit opt-in because the 768-ply fixture is intentionally compute-heavy.
import { createHash } from 'node:crypto'
import { createRequire, registerHooks } from 'node:module'
import { spawnSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { dirname, join, resolve } from 'node:path'
import { performance } from 'node:perf_hooks'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const BASELINE = '/private/tmp/kchess-benchmark-ts'
const WASM_PATH = resolve(
  process.env.KCHESS_BRIDGE_BENCH_WASM_PATH ??
    join(ROOT, 'apps/desktop/app/assets/rules/kchess.wasm'),
)
const SIZES = [128, 384, 768]
const RUNS = 7
const ITERATIONS = Number(process.env.KCHESS_BRIDGE_BENCH_ITERATIONS ?? 20)
const WARMUPS = Number(process.env.KCHESS_BRIDGE_BENCH_WARMUPS ?? 2)
const encoder = new TextEncoder()

function fixture(plies) {
  const sans = ['Nf3', 'Nf6', 'Ng1', 'Ng8']
  const body = []
  for (let ply = 0; ply < plies; ply += 2) {
    const fullMove = Math.floor(ply / 2) + 1
    body.push(`${fullMove}. ${sans[ply % 4]} ${sans[(ply + 1) % 4]}`)
  }
  const pgn = `[Event "Bridge scaling"]\n[Result "*"]\n\n${body.join(' ')} *`
  const pathMoves = Array.from(
    { length: plies },
    (_, ply) => ['g1f3', 'g8f6', 'f3g1', 'f6g8'][ply % 4],
  )
  return { pgn, path: pathMoves.join(' '), pathMoves }
}

const digest = (value) => createHash('sha256').update(JSON.stringify(value)).digest('hex')

async function load(mode) {
  if (mode === 'typescript') {
    registerHooks({
      resolve(specifier, context, nextResolve) {
        if (
          context.parentURL?.startsWith(pathToFileURL(BASELINE).href) &&
          specifier.startsWith('.') &&
          !/\.[cm]?[jt]sx?$/.test(specifier)
        )
          return nextResolve(`${specifier}.ts`, context)
        return nextResolve(specifier, context)
      },
    })
    const tree = await import(pathToFileURL(join(BASELINE, 'core/src/domain/analysisTree.ts')))
    return { tree, calls: undefined }
  }

  if (mode !== 'wasm' && mode !== 'native') throw new Error(`Unknown implementation ${mode}`)
  const engine = await import(pathToFileURL(join(ROOT, 'crates/kchess-wasm/js/dist/engine.js')))
  const tree = await import(pathToFileURL(join(ROOT, 'crates/kchess-wasm/js/dist/analysisTree.js')))
  const calls = []
  let invoke
  if (mode === 'wasm') {
    const wasm = await WebAssembly.instantiate(readFileSync(WASM_PATH))
    invoke = engine.wasmBinding(wasm.instance.exports).invoke
  } else {
    const native = createRequire(import.meta.url)(join(ROOT, 'crates/kchess-node/index.cjs'))
    invoke = (method, args) => native.invoke(method, args)
  }
  engine.setRulesBinding({
    invoke(method, args) {
      const result = invoke(method, args)
      calls.push({
        method,
        requestBytes: encoder.encode(args).length,
        responseBytes: encoder.encode(result).length,
      })
      return result
    },
  })
  return { tree, calls }
}

function runTask(api, task, data, preparedRoot) {
  api.calls?.splice(0)
  if (task === 'treeFromPgn') {
    const value = api.tree.treeFromPgn(data.pgn)
    if (!value) throw new Error('The full-PGN import rejected a fixture.')
    return value
  }
  const root = preparedRoot
  if (task === 'treeToPgn') return api.tree.treeToPgn(root)
  if (task === 'navigate') {
    const nodes = api.tree.nodesAlong(root, data.path)
    const reconstructed = api.tree.pathOf(nodes.slice(1).map((node) => node.uci))
    if (nodes.length !== data.pathMoves.length + 1)
      throw new Error('Fixture path did not resolve fully.')
    return { reconstructed, nodes: nodes.length }
  }
  if (task === 'deepEdit') {
    const added = api.tree.addMove(root, data.path, 'd2d4')
    if (added !== `${data.path} d2d4`)
      throw new Error('Deep edit did not return the expected path.')
    return { added }
  }
  throw new Error(`Unknown task ${task}`)
}

async function child(mode, action) {
  const api = await load(mode)
  const rows = []
  for (const plies of SIZES) {
    const data = fixture(plies)
    const results = {}
    const traces = {}
    for (const task of ['treeFromPgn', 'treeToPgn', 'navigate', 'deepEdit']) {
      const root = task === 'treeFromPgn' ? undefined : api.tree.treeFromPgn(data.pgn)
      if (task !== 'treeFromPgn' && !root)
        throw new Error('The full-PGN import rejected a fixture.')
      const output = runTask(api, task, data, root)
      results[task] = output
      traces[task] = api.calls ? [...api.calls] : []
    }
    rows.push({
      plies,
      pgnBytes: encoder.encode(data.pgn).length,
      treeBytes: encoder.encode(JSON.stringify(results.treeFromPgn)).length,
      pgnRoundTripBytes: encoder.encode(results.treeToPgn).length,
      digest: Object.fromEntries(
        Object.entries(results).map(([task, output]) => [task, digest(output)]),
      ),
      traces,
    })
  }
  if (action === 'verify') {
    process.stdout.write(JSON.stringify({ mode, rows }))
    return
  }
  if (action !== 'measure') throw new Error(`Unknown action ${action}`)
  for (const row of rows) {
    const data = fixture(row.plies)
    const roots = Object.fromEntries(
      ['treeToPgn', 'navigate', 'deepEdit'].map((task) => [task, api.tree.treeFromPgn(data.pgn)]),
    )
    if (Object.values(roots).some((root) => !root))
      throw new Error('The full-PGN import rejected a fixture.')
    const apiTask = (task) => runTask(api, task, data, roots[task])
    row.timing = {}
    for (const task of ['treeFromPgn', 'treeToPgn', 'navigate', 'deepEdit']) {
      for (let index = 0; index < WARMUPS; index++) apiTask(task)
      const start = performance.now()
      for (let index = 0; index < ITERATIONS; index++) apiTask(task)
      row.timing[task] = { iterations: ITERATIONS, elapsedMs: performance.now() - start }
    }
  }
  process.stdout.write(JSON.stringify({ mode, rows }))
}

function fresh(mode, action) {
  const result = spawnSync(
    process.execPath,
    ['--experimental-strip-types', fileURLToPath(import.meta.url), '--child', mode, action],
    { cwd: ROOT, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 },
  )
  if (result.status !== 0)
    throw new Error(
      `${mode} ${action} failed (${result.status}): ${result.stderr || result.stdout}`,
    )
  return JSON.parse(result.stdout)
}

const args = process.argv.slice(2)
if (args[0] === '--child') {
  await child(args[1], args[2])
} else if (args[0] === '--verify' || args[0] === '--measure') {
  const action = args[0].slice(2)
  const sameOutputs = (results, plies, candidate) => {
    const sizeIndex = SIZES.indexOf(plies)
    const expected = JSON.stringify(results.typescript.rows[sizeIndex].digest)
    const actual = JSON.stringify(results[candidate].rows[sizeIndex].digest)
    if (expected !== actual) throw new Error(`Output mismatch at ${plies} plies for ${candidate}.`)
  }
  let implementations
  let comparisons
  if (action === 'verify') {
    implementations = Object.fromEntries(
      ['typescript', 'wasm', 'native'].map((mode) => [mode, fresh(mode, action)]),
    )
    for (const plies of SIZES) {
      const results = Object.fromEntries(
        Object.entries(implementations).map(([mode, result]) => [mode, result]),
      )
      sameOutputs(results, plies, 'wasm')
      sameOutputs(results, plies, 'native')
    }
  } else {
    comparisons = []
    for (const candidate of ['wasm', 'native']) {
      const pairs = []
      for (let pair = 0; pair < RUNS; pair++) {
        const order = pair % 2 === 0 ? ['typescript', candidate] : [candidate, 'typescript']
        const results = Object.fromEntries(order.map((mode) => [mode, fresh(mode, action)]))
        for (const plies of SIZES) sameOutputs(results, plies, candidate)
        pairs.push({ pair: pair + 1, order, results })
      }
      comparisons.push({ candidate, pairs })
    }
  }
  const output = {
    kind: `bridge-hotspots-${action}`,
    endpoints: {
      typescript: 'f479f7ce10d7363df7132d616d8ad71b775ec7a7',
      rust: '8f58da7f4d51371001e95d1c33cbd51ed9ad20c2',
    },
    process: process.version,
    os: process.platform,
    arch: process.arch,
    wasmArtifact:
      process.env.KCHESS_BRIDGE_BENCH_WASM_PATH ?? 'apps/desktop/app/assets/rules/kchess.wasm',
    sizes: SIZES,
    iterationsPerTask: action === 'measure' ? ITERATIONS : 0,
    warmupsPerTask: action === 'measure' ? WARMUPS : 0,
    ...(action === 'verify' ? { implementations } : { pairsPerCandidate: RUNS, comparisons }),
    interpretation:
      'WASM/native traces count rules.invoke calls and UTF-8 JSON bytes at that boundary; timing includes the public analysisTree helper.',
  }
  const destination = args[1]
  if (destination) writeFileSync(resolve(destination), `${JSON.stringify(output, null, 2)}\n`)
  process.stdout.write(`${JSON.stringify(output, null, 2)}\n`)
} else {
  process.stderr.write(
    'Usage: node tooling/benchmarks/bridge-hotspots.mjs --verify [output.json]\n       node tooling/benchmarks/bridge-hotspots.mjs --measure [output.json] (wait for a quiet-host signal)\n',
  )
  process.exitCode = 2
}
