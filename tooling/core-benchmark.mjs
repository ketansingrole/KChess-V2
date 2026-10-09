import { spawnSync } from 'node:child_process'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { build } from 'vite'

/**
 * The Rust rules on the seeded fixtures, compared with the TypeScript rules' recorded numbers
 * (tooling/benchmark-baseline/typescript.json; the TypeScript rules no longer exist to re-run).
 * Writes test-results/guardrails/core-benchmark.{json,md}.
 */
const root = fileURLToPath(new URL('..', import.meta.url))
await build({
  configFile: false,
  root,
  logLevel: 'warn',
  ssr: { noExternal: true },
  build: {
    ssr: 'tooling/core-benchmark.ts',
    target: 'node24',
    // Inside the core so `@kchess/native` resolves exactly as it does for the built core.
    outDir: 'core/node_modules/.cache/kchess-benchmark',
    emptyOutDir: true,
    rollupOptions: { output: { format: 'es', entryFileNames: 'core-benchmark.mjs' } },
  },
})
const bundle = `${root}core/node_modules/.cache/kchess-benchmark/core-benchmark.mjs`
const results = 'test-results/guardrails'
mkdirSync(results, { recursive: true })
const node = (args) => {
  const run = spawnSync(process.execPath, args, { cwd: root, stdio: 'inherit' })
  if (run.status !== 0) process.exit(run.status ?? 1)
}

const baseline = JSON.parse(
  readFileSync(new URL('benchmark-baseline/typescript.json', import.meta.url), 'utf8'),
)
node([bundle, '--out', `${results}/core-benchmark-native.json`])
const native = JSON.parse(readFileSync(`${results}/core-benchmark-native.json`, 'utf8'))
if (JSON.stringify(native.fixtures) !== JSON.stringify(baseline.fixtures))
  throw new Error('The fixtures changed; the recorded TypeScript numbers no longer apply.')
for (const [key, recorded] of Object.entries(baseline.results))
  if (native.results[key]?.result !== recorded.result)
    throw new Error(
      `${key}: TypeScript counted ${recorded.result}, Rust ${native.results[key]?.result}.`,
    )

// The same fixtures through the built core's public API.
const fixtures = 'test-results/benchmark-fixtures'
node([bundle, '--fixtures', fixtures])
const pnpm = spawnSync('pnpm', ['--filter', '@kchess/core', 'build'], {
  cwd: root,
  stdio: 'inherit',
  shell: process.platform === 'win32',
})
if (pnpm.status !== 0) process.exit(pnpm.status ?? 1)
node(['tooling/core-library-benchmark.mjs', fixtures, `${results}/core-api-native.json`])
const api = JSON.parse(readFileSync(`${results}/core-api-native.json`, 'utf8'))
if (JSON.stringify(api.counts) !== JSON.stringify(baseline.api.counts))
  throw new Error(
    `Core API loaded ${JSON.stringify(api.counts)}; TypeScript loaded ${JSON.stringify(baseline.api.counts)}.`,
  )
if (!api.rules.startsWith('native'))
  throw new Error('The core API run did not load the Rust rules.')

const ms = (n) => `${n.toFixed(n < 10 ? 2 : 1)} ms`
const rows = [
  ['Core API: first library() of a fresh core', baseline.api.libraryFirstMs, api.libraryFirstMs],
  ['Core API: library()', baseline.api.libraryMs, api.libraryMs],
  ['Core API: rename a study', baseline.api.studyRenameMs, api.studyRenameMs],
  ['Library load, first in a fresh process', baseline.coldLibraryLoadMs, native.coldLibraryLoadMs],
  ...Object.keys(baseline.results).map((key) => [
    key,
    baseline.results[key].medianMs,
    native.results[key].medianMs,
  ]),
]
const table = [
  '| Workload | TypeScript (recorded) | Rust | Speed-up |',
  '| --- | ---: | ---: | ---: |',
  ...rows.map(
    ([name, before, after]) =>
      `| ${name} | ${ms(before)} | ${ms(after)} | ${(before / after).toFixed(1)}× |`,
  ),
].join('\n')
const markdown = `# Core benchmark\n\nFixtures: ${JSON.stringify(native.fixtures)}\n\n${baseline.note}\n\n${table}\n`
writeFileSync(
  `${results}/core-benchmark.json`,
  JSON.stringify({ baseline, native, api }, null, 2) + '\n',
)
writeFileSync(`${results}/core-benchmark.md`, markdown)
console.info('\n' + table)
