// Frame benchmarks of the GPUI app: runs `KChess` builds made with `--features bench`
// (apps/gpui/src/bench.rs) through each scenario on a fresh throwaway profile, alternating the
// builds run by run, and writes every run plus the per-build medians as JSON.
//
//   node tooling/gpui-render-benchmark.mjs --build before=/path/KChess --build after=/path/KChess \
//     [--scenario local-clock ...] [--runs 5] [--seconds 30] [--out file.json]
//
// It never sends input to the desktop: each scenario drives the app through its own pages.
import { spawn } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const SCENARIOS = [
  'home-idle',
  'local-clock',
  'board-animation',
  'analysis-engine',
  'openings-shell',
]
const ENGINE = resolve('target/gpui-package/stockfish/stockfish')

const args = process.argv.slice(2)
const option = (name) => {
  const values = []
  for (let index = 0; index < args.length; index++)
    if (args[index] === `--${name}`) values.push(args[++index])
  return values
}
const builds = option('build').map((value) => {
  const [label, path] = value.split('=')
  if (!label || !path || !existsSync(path)) throw new Error(`--build label=path: ${value}`)
  return { label, path: resolve(path) }
})
if (builds.length === 0) throw new Error('at least one --build label=path')
const scenarios = option('scenario').length > 0 ? option('scenario') : SCENARIOS
const runs = Number(option('runs')[0] ?? 5)
const seconds = Number(option('seconds')[0] ?? 30)
const out = option('out')[0] ?? 'gpui-render-benchmark.json'

function runOnce(build, scenario) {
  const profile = mkdtempSync(join(tmpdir(), 'kchess-bench-'))
  const report = join(profile, 'report.json')
  return new Promise((done, fail) => {
    const child = spawn(build.path, [], {
      stdio: ['ignore', 'ignore', 'pipe'],
      env: {
        ...process.env,
        KCHESS_USER_DATA_DIR: join(profile, 'data'),
        KCHESS_ENGINE_PATH: ENGINE,
        KCHESS_BENCH: scenario,
        KCHESS_BENCH_SECONDS: String(seconds),
        KCHESS_BENCH_OUT: report,
      },
    })
    let stderr = ''
    child.stderr.on('data', (chunk) => (stderr += chunk))
    // Startup, warm-up and quitting take well under a minute; stop only this run's own process.
    const limit = setTimeout(() => child.kill('SIGKILL'), (seconds + 60) * 1000)
    child.on('exit', (code, signal) => {
      clearTimeout(limit)
      try {
        if (!existsSync(report))
          return fail(
            new Error(`${build.label} ${scenario}: no report (${code ?? signal})\n${stderr}`),
          )
        done(JSON.parse(readFileSync(report, 'utf8')))
      } finally {
        rmSync(profile, { recursive: true, force: true })
      }
    })
  })
}

const median = (values) => {
  const sorted = values.filter((value) => typeof value === 'number').sort((a, b) => a - b)
  if (sorted.length === 0) return null
  const middle = Math.floor(sorted.length / 2)
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2
}
const METRICS = [
  'draws_per_s',
  'draw_ms_p50',
  'draw_ms_p95',
  'draw_ms_max',
  'draw_ms_per_s',
  'request_layout_ms_per_s',
  'prepaint_ms_per_s',
  'paint_ms_per_s',
  'main_thread_busy_fraction',
  'process_cpu_fraction',
  'max_rss_mib',
]

const result = { seconds, runs, builds, scenarios: {} }
for (const scenario of scenarios) {
  const samples = Object.fromEntries(builds.map((build) => [build.label, []]))
  for (let run = 0; run < runs; run++) {
    // Alternate the order so drift (thermals, background work) falls on every build alike.
    const order = run % 2 ? [...builds].reverse() : builds
    for (const build of order) {
      const report = await runOnce(build, scenario)
      samples[build.label].push(report)
      console.log(
        `${scenario} ${build.label} #${run + 1}: ${report.draws_per_s.toFixed(1)} draws/s, ` +
          `p50 ${report.draw_ms_p50.toFixed(2)} ms, draw ${report.draw_ms_per_s.toFixed(1)} ms/s, ` +
          `cpu ${((report.process_cpu_fraction ?? 0) * 100).toFixed(1)}%`,
      )
    }
  }
  const medians = Object.fromEntries(
    builds.map((build) => [
      build.label,
      Object.fromEntries(
        METRICS.map((metric) => [metric, median(samples[build.label].map((r) => r[metric]))]),
      ),
    ]),
  )
  result.scenarios[scenario] = { medians, samples }
}
writeFileSync(out, JSON.stringify(result, null, 2) + '\n')
console.log(`wrote ${out}`)
