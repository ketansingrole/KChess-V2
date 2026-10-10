import { spawn } from 'node:child_process'
import { mkdir, readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'

/**
 * Coverage floors for the Rust workspace, as whole percentages rounded down from the measured
 * totals of `cargo llvm-cov --workspace` (2026-10-10: lines 62.83%, regions 58.63%, functions
 * 60.39%). The run fails below a floor. Ratchet them up when the measured totals rise; do not lower
 * them without a measurement that explains the drop.
 */
export const FLOORS = { lines: 62, regions: 58, functions: 60 }

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

function runCargoLlvmCov(args) {
  return new Promise((resolve) => {
    const child = spawn('cargo', ['llvm-cov', ...args], { stdio: 'inherit' })
    child.once('error', (cause) => {
      console.error(cause)
      resolve(1)
    })
    child.once('exit', (code) => resolve(code ?? 1))
  })
}

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
