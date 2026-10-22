import { spawn } from 'node:child_process'
import { mkdir, readFile, readdir } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * Coverage floors for the Rust workspace, as whole percentages rounded down from the measured
 * totals of `cargo llvm-cov --workspace` (2026-10-17, after the argument-depth, panic-hook and
 * task-guard tests: lines 66.82%, regions 63.29%, functions 63.99%). The run fails below a floor.
 * Ratchet them up when the measured totals rise; do not lower them without a measurement that
 * explains the drop.
 *
 * The GPUI app (`apps/gpui`) is left out: like the Electron renderer, its views are checked by
 * running the app, and its own unit tests still run in `cargo test --workspace`. With it, the
 * totals fell to 60.89% of lines because its 8,301 lines of drawing code are 7.5% covered; the
 * rest of the workspace measured 71.79% (2026-10-23).
 */
export const FLOORS = { lines: 66, regions: 63, functions: 63 }

/** The summary's totals, as `{ lines, regions, functions }` percentages. */
export function percentages(report) {
  const totals = report?.data?.[0]?.totals
  if (!totals) throw new Error('The coverage report has no totals.')
  return Object.fromEntries(
    Object.keys(FLOORS).map((metric) => {
      const percent = totals[metric]?.percent
      if (typeof percent !== 'number') throw new Error(`The coverage report has no ${metric}.`)
      return [metric, percent]
    }),
  )
}

/** The metrics below their floors, as messages. */
export function belowFloors(measured, floors = FLOORS) {
  return Object.keys(floors)
    .filter((metric) => measured[metric] < floors[metric])
    .map(
      (metric) =>
        `${metric} coverage ${measured[metric].toFixed(2)}% is below the ${floors[metric]}% floor`,
    )
}

/** Whether any Rust doc comment holds a code block: doc-tests, which llvm-cov does not run on stable. */
export async function hasDocTests(directory = 'crates') {
  const docBlock = /^\s*\/\/[/!].*```/m
  for (const entry of await readdir(directory, { withFileTypes: true, recursive: true })) {
    if (!entry.isFile() || !entry.name.endsWith('.rs')) continue
    const path = join(entry.parentPath, entry.name)
    if (path.includes('/target/') || path.includes('node_modules')) continue
    if (docBlock.test(await readFile(path, 'utf8'))) return true
  }
  return false
}

function runCargo(args) {
  return new Promise((resolve) => {
    const child = spawn('cargo', args, { stdio: 'inherit' })
    child.once('error', (cause) => {
      console.error(cause)
      resolve(1)
    })
    child.once('exit', (code) => resolve(code ?? 1))
  })
}
const runCargoLlvmCov = (args) => runCargo(['llvm-cov', ...args])

export async function checkRustCoverage(outputDirectory = 'test-results/rust-coverage') {
  const available = await runCargoLlvmCov(['--version'])
  if (available !== 0) {
    console.error(
      '[kchess] cargo-llvm-cov is not available: run `cargo install cargo-llvm-cov --locked` and `rustup component add llvm-tools-preview`.',
    )
    return 1
  }
  await mkdir(outputDirectory, { recursive: true })
  const output = `${outputDirectory}/summary.json`
  const code = await runCargoLlvmCov([
    '--workspace',
    '--exclude',
    'kchess-gpui',
    '--locked',
    '--summary-only',
    '--json',
    '--output-path',
    output,
  ])
  if (code !== 0) {
    console.error('[kchess] The Rust tests failed under coverage.')
    return code
  }
  // This run is `check`'s Rust test run, so doc-tests (outside llvm-cov on stable) run here too.
  if (await hasDocTests()) {
    const docs = await runCargo(['test', '--doc', '--workspace', '--locked'])
    if (docs !== 0) {
      console.error('[kchess] The Rust doc-tests failed.')
      return docs
    }
  }
  const measured = percentages(JSON.parse(await readFile(output, 'utf8')))
  console.info(
    `[kchess] Rust coverage: lines ${measured.lines.toFixed(2)}%, regions ${measured.regions.toFixed(2)}%, functions ${measured.functions.toFixed(2)}% (floors ${FLOORS.lines}/${FLOORS.regions}/${FLOORS.functions}).`,
  )
  const failures = belowFloors(measured)
  for (const failure of failures) console.error(`[kchess] ${failure}.`)
  return failures.length ? 1 : 0
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exitCode = await checkRustCoverage()
}
