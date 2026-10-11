#!/usr/bin/env node
// Paired, fresh-process comparison of the pre-migration TS rules and current renderer WASM rules.
// `--verify` checks matching outputs only; `--measure` performs seven paired timing runs.
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
  process.env.KCHESS_RULES_BENCH_WASM_PATH ??
    join(ROOT, 'apps/desktop/app/assets/rules/kchess.wasm'),
)
const RUNS = 7
const ITERATIONS = Number(process.env.KCHESS_RULES_BENCH_ITERATIONS ?? 200)
const WARMUPS = Number(process.env.KCHESS_RULES_BENCH_WARMUPS ?? 5)

const MOVES = [
  'e2e4',
  'e7e5',
  'g1f3',
  'b8c6',
  'f1b5',
  'a7a6',
  'b5a4',
  'g8f6',
  'e1h1',
  'f8e7',
  'f1e1',
  'b7b5',
  'a4b3',
  'd7d6',
  'c2c3',
  'e8h8',
  'h2h3',
  'c6b8',
  'd2d4',
  'b8d7',
  'c3c4',
  'c7c6',
  'b1c3',
  'c8b7',
  'a2a3',
  'd8c7',
  'b3a2',
  'f8e8',
  'c1e3',
  'e7f8',
  'a1c1',
  'a8c8',
  'b2b4',
  'c7b8',
]
const START_FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const FEN = 'r2q1rk1/1bpnbppp/p1p1pn2/1p2p3/1P1PP3/P1N1BN1P/2Q2PP1/R1B1R1K1 w - - 0 12'
const PGN = `[Event "Rules benchmark"]\n[Site "Local"]\n[Result "*"]\n\n1. e4 e5 2. Nf3 Nc6 3. Bb5 a6 (3... Nf6 4. O-O) 4. Ba4 Nf6 5. O-O Be7 6. Re1 b5 7. Bb3 d6 8. c3 O-O 9. h3 Nb8 10. d4 Nbd7 11. c4 c6 12. Nc3 Bb7 13. a3 Qc7 14. Ba2 Rfe8 15. Be3 Bf8 16. Rc1 Rac8 17. b4 Qb8 *`
const TASKS = ['pgnMainlineFirstGame', 'sanHistory', 'legalDestinations', 'replayPositions']
const PGN_TREE_TASKS = ['pgnTreeImport']

const digest = (value) => createHash('sha256').update(JSON.stringify(value)).digest('hex')
const sortedDestinations = (dests) =>
  [...dests.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([from, tos]) => [from, [...tos].sort()])

async function load(mode, tasks) {
  if (mode === 'typescript') {
    // This extracted baseline uses extensionless ESM imports. The old Vite/Vitest
    // toolchain resolved those; reproduce that resolution for direct Node imports.
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
    const chess = await import(pathToFileURL(join(BASELINE, 'core/src/domain/chess.ts')))
    const review = await import(pathToFileURL(join(BASELINE, 'core/src/domain/review.ts')))
    const analysisTree = tasks.includes('pgnTreeImport')
      ? await import(pathToFileURL(join(BASELINE, 'core/src/domain/analysisTree.ts')))
      : undefined
    const pgn = await import(pathToFileURL(join(BASELINE, 'node_modules/chessops/dist/esm/pgn.js')))
    return {
      pgnMainlineFirstGame() {
        const games = pgn.parsePgn(PGN)
        const game = games[0]
        return {
          headers: [...game.headers.entries()],
          sans: [...game.moves.mainline()].map((move) => move.san),
        }
      },
      sanHistory() {
        return chess.sanHistory(MOVES)
      },
      legalDestinations() {
        const position = chess.positionFromFen(FEN)
        if (!position) throw new Error('Baseline could not parse the fixed legal-move FEN.')
        return sortedDestinations(chess.destsFor(position))
      },
      replayPositions() {
        return review.replay(START_FEN, MOVES)
      },
      pgnTreeImport() {
        return analysisTree.treeFromPgn(PGN)
      },
    }
  }

  if (mode !== 'wasm' && mode !== 'native') throw new Error(`Unknown rules implementation: ${mode}`)
  const engine = await import(pathToFileURL(join(ROOT, 'crates/kchess-wasm/js/dist/engine.js')))
  const [chess, pgn, review, position, analysisTree] = await Promise.all([
    import(pathToFileURL(join(ROOT, 'crates/kchess-wasm/js/dist/chess.js'))),
    import(pathToFileURL(join(ROOT, 'crates/kchess-wasm/js/dist/pgn.js'))),
    import(pathToFileURL(join(ROOT, 'crates/kchess-wasm/js/dist/review.js'))),
    import(pathToFileURL(join(ROOT, 'crates/kchess-wasm/js/dist/position.js'))),
    tasks.includes('pgnTreeImport')
      ? import(pathToFileURL(join(ROOT, 'crates/kchess-wasm/js/dist/analysisTree.js')))
      : Promise.resolve(undefined),
  ])
  if (mode === 'wasm') {
    const wasm = await WebAssembly.instantiate(readFileSync(WASM_PATH))
    engine.setRulesBinding(engine.wasmBinding(wasm.instance.exports))
  } else {
    const native = createRequire(import.meta.url)(join(ROOT, 'crates/kchess-node/index.cjs'))
    engine.setRulesBinding({ invoke: (method, args) => native.invoke(method, args) })
  }
  return {
    pgnMainlineFirstGame() {
      const line = pgn.pgnMainline(PGN)
      if (!line) throw new Error('WASM parser returned no mainline for the fixed PGN.')
      return { headers: line.headers, sans: line.moves.map((move) => move.san) }
    },
    sanHistory() {
      return chess.sanHistory(MOVES)
    },
    legalDestinations() {
      const pos = position.Position.fromFen(FEN)
      if (!pos) throw new Error('WASM rules could not parse the fixed legal-move FEN.')
      return sortedDestinations(pos.dests('board'))
    },
    replayPositions() {
      return review.replay(START_FEN, MOVES)
    },
    pgnTreeImport() {
      return analysisTree.treeFromPgn(PGN)
    },
  }
}

async function child(mode, action, tasks = TASKS) {
  const implementation = await load(mode, tasks)
  const outputs = Object.fromEntries(tasks.map((task) => [task, implementation[task]()]))
  if (
    ('sanHistory' in outputs && outputs.sanHistory.length !== MOVES.length) ||
    ('replayPositions' in outputs && outputs.replayPositions.length !== MOVES.length + 1) ||
    ('pgnMainlineFirstGame' in outputs &&
      outputs.pgnMainlineFirstGame.sans.length !== MOVES.length) ||
    ('pgnTreeImport' in outputs && !outputs.pgnTreeImport)
  )
    throw new Error(`${mode} fixture must replay all ${MOVES.length} plies.`)
  if (action === 'verify') {
    process.stdout.write(
      JSON.stringify({
        mode,
        outputs,
        digests: Object.fromEntries(tasks.map((task) => [task, digest(outputs[task])])),
      }),
    )
    return
  }
  if (action !== 'measure') throw new Error(`Unknown child action: ${action}`)
  const timings = {}
  for (const task of tasks) {
    for (let i = 0; i < WARMUPS; i++) implementation[task]()
    const start = performance.now()
    for (let i = 0; i < ITERATIONS; i++) implementation[task]()
    timings[task] = { iterations: ITERATIONS, elapsedMs: performance.now() - start }
  }
  process.stdout.write(
    JSON.stringify({
      mode,
      digests: Object.fromEntries(tasks.map((task) => [task, digest(outputs[task])])),
      timings,
    }),
  )
}

function runFresh(mode, action, tasks = TASKS) {
  const result = spawnSync(
    process.execPath,
    [
      '--experimental-strip-types',
      fileURLToPath(import.meta.url),
      '--child',
      mode,
      action,
      ...tasks,
    ],
    {
      cwd: ROOT,
      encoding: 'utf8',
      maxBuffer: 8 * 1024 * 1024,
    },
  )
  if (result.status !== 0)
    throw new Error(
      `${mode} ${action} subprocess failed (${result.status}): ${result.stderr || result.stdout}`,
    )
  return JSON.parse(result.stdout)
}

const args = process.argv.slice(2)
if (args[0] === '--child') {
  await child(args[1], args[2], args.slice(3).length ? args.slice(3) : TASKS)
} else if (args[0] === '--verify') {
  const typescript = runFresh('typescript', 'verify')
  const wasm = runFresh('wasm', 'verify')
  const native = runFresh('native', 'verify')
  for (const task of TASKS) {
    if (
      typescript.digests[task] !== wasm.digests[task] ||
      typescript.digests[task] !== native.digests[task]
    )
      throw new Error(
        `Correctness mismatch for ${task}: TypeScript ${typescript.digests[task]}, WASM ${wasm.digests[task]}, native ${native.digests[task]}`,
      )
  }
  const output = {
    kind: 'rules-correctness-only',
    endpointBaseline: 'f479f7ce10d7363df7132d616d8ad71b775ec7a7',
    endpointRust: '8f58da7f4d51371001e95d1c33cbd51ed9ad20c2',
    wasmArtifact:
      process.env.KCHESS_RULES_BENCH_WASM_PATH ??
      'apps/desktop/app/assets/rules/kchess.wasm (pre-existing; no build performed)',
    tasks: TASKS.map((task) => ({
      task,
      digest: typescript.digests[task],
      match: true,
      implementationDigests: {
        typescript: typescript.digests[task],
        wasm: wasm.digests[task],
        native: native.digests[task],
      },
      output: typescript.outputs[task],
    })),
    timing: null,
  }
  const destination = args[1]
  if (destination) writeFileSync(resolve(destination), `${JSON.stringify(output, null, 2)}\n`)
  process.stdout.write(`${JSON.stringify(output, null, 2)}\n`)
} else if (args[0] === '--measure') {
  const comparisons = []
  for (const candidate of ['wasm', 'native']) {
    const pairs = []
    for (let pair = 0; pair < RUNS; pair++) {
      const order = pair % 2 === 0 ? ['typescript', candidate] : [candidate, 'typescript']
      const results = order.map((mode) => runFresh(mode, 'measure'))
      const [left, right] = results
      for (const task of TASKS) {
        if (left.digests[task] !== right.digests[task])
          throw new Error(
            `Correctness mismatch during ${candidate} timing pair ${pair + 1}, ${task}.`,
          )
      }
      pairs.push({ pair: pair + 1, order, results })
    }
    comparisons.push({ candidate, pairs })
  }
  const output = {
    kind: 'rules-timing',
    endpointBaseline: 'f479f7ce10d7363df7132d616d8ad71b775ec7a7',
    endpointRust: '8f58da7f4d51371001e95d1c33cbd51ed9ad20c2',
    process: process.version,
    os: process.platform,
    arch: process.arch,
    wasmArtifact:
      process.env.KCHESS_RULES_BENCH_WASM_PATH ?? 'apps/desktop/app/assets/rules/kchess.wasm',
    nativeArtifact: `crates/kchess-node/kchess-native.${process.platform}-${process.arch}.node`,
    iterationsPerTask: ITERATIONS,
    warmupIterationsPerTask: WARMUPS,
    warmState:
      'module imports and WASM/native binding initialization excluded; operation-level wrapper included',
    comparisons,
  }
  const destination = args[1]
  if (destination) writeFileSync(resolve(destination), `${JSON.stringify(output, null, 2)}\n`)
  process.stdout.write(`${JSON.stringify(output, null, 2)}\n`)
} else if (args[0] === '--verify-pgn-tree') {
  const tasks = PGN_TREE_TASKS
  const implementations = Object.fromEntries(
    await Promise.all(
      ['typescript', 'wasm', 'native'].map(async (mode) => [mode, runFresh(mode, 'verify', tasks)]),
    ),
  )
  const digestByMode = Object.fromEntries(
    Object.entries(implementations).map(([mode, result]) => [mode, result.digests.pgnTreeImport]),
  )
  if (
    digestByMode.typescript !== digestByMode.wasm ||
    digestByMode.typescript !== digestByMode.native
  )
    throw new Error(`PGN tree output mismatch: ${JSON.stringify(digestByMode)}`)
  const output = {
    kind: 'rules-pgn-tree-correctness-only',
    endpointBaseline: 'f479f7ce10d7363df7132d616d8ad71b775ec7a7',
    endpointRust: '8f58da7f4d51371001e95d1c33cbd51ed9ad20c2',
    pgnFixture: 'one 34-ply main line with a two-ply variation; imports full analysis tree',
    implementations: Object.fromEntries(
      Object.entries(implementations).map(([mode, result]) => [
        mode,
        { digest: result.digests.pgnTreeImport, output: result.outputs.pgnTreeImport },
      ]),
    ),
    match: true,
    timing: null,
  }
  const destination = args[1]
  if (destination) writeFileSync(resolve(destination), `${JSON.stringify(output, null, 2)}\n`)
  process.stdout.write(`${JSON.stringify(output, null, 2)}\n`)
} else if (args[0] === '--measure-pgn-tree') {
  const tasks = PGN_TREE_TASKS
  const comparisons = []
  for (const candidate of ['wasm', 'native']) {
    const pairs = []
    for (let pair = 0; pair < RUNS; pair++) {
      const order = pair % 2 === 0 ? ['typescript', candidate] : [candidate, 'typescript']
      const results = order.map((mode) => runFresh(mode, 'measure', tasks))
      const [left, right] = results
      if (left.digests.pgnTreeImport !== right.digests.pgnTreeImport)
        throw new Error(`PGN tree output mismatch during ${candidate} timing pair ${pair + 1}.`)
      pairs.push({ pair: pair + 1, order, results })
    }
    comparisons.push({ candidate, pairs })
  }
  const output = {
    kind: 'rules-pgn-tree-timing',
    endpointBaseline: 'f479f7ce10d7363df7132d616d8ad71b775ec7a7',
    endpointRust: '8f58da7f4d51371001e95d1c33cbd51ed9ad20c2',
    process: process.version,
    os: process.platform,
    arch: process.arch,
    iterationsPerTask: ITERATIONS,
    warmupIterationsPerTask: WARMUPS,
    warmState:
      'module imports and WASM/native binding initialization excluded; full analysis-tree import wrapper included',
    pgnFixture: 'one 34-ply main line with a two-ply variation; imports full analysis tree',
    comparisons,
  }
  const destination = args[1]
  if (destination) writeFileSync(resolve(destination), `${JSON.stringify(output, null, 2)}\n`)
  process.stdout.write(`${JSON.stringify(output, null, 2)}\n`)
} else {
  process.stderr.write(
    'Usage: node tooling/benchmarks/rules-benchmark.mjs --verify [output.json]\n       node tooling/benchmarks/rules-benchmark.mjs --measure [output.json] (wait for quiet signal)\n       node tooling/benchmarks/rules-benchmark.mjs --verify-pgn-tree [output.json]\n       node tooling/benchmarks/rules-benchmark.mjs --measure-pgn-tree [output.json] (wait for quiet signal)\n',
  )
  process.exitCode = 2
}
