// Eight simultaneous profile reads of the same user: one shared wire request after coalescing,
// eight independent requests before it.
// Usage: node perf/bench-network.mjs --root=/path/to/repo --label=before
import { createServer } from 'node:http'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { performance } from 'node:perf_hooks'

const args = Object.fromEntries(
  process.argv
    .slice(2)
    .filter((a) => a.startsWith('--'))
    .map((a) => {
      const [k, v] = a.replace(/^--/, '').split('=')
      return [k, v ?? 'true']
    }),
)
const root = resolve(args.root ?? resolve(import.meta.dirname, '..'))
const label = args.label ?? 'run'
const runs = Number(args.runs ?? 3)
const calls = Number(args.calls ?? 8)
const delay = Number(args.delay ?? 100)

let requests = 0
const server = createServer((request, response) => {
  requests++
  setTimeout(() => {
    const username = decodeURIComponent(
      new URL(request.url ?? '/', 'http://x').pathname.replace('/api/user/', ''),
    )
    response.writeHead(200, { 'content-type': 'application/json', 'cache-control': 'no-store' })
    response.end(JSON.stringify({ username, id: username.toLowerCase(), title: 'IM' }))
  }, delay)
})
await new Promise((done) => server.listen(0, '127.0.0.1', done))
const fixture = `http://127.0.0.1:${server.address().port}`

process.env.KCHESS_STORE_DEBUG = '1'
process.env.KCHESS_TEST_LICHESS_BASE = fixture
const { createKChessCore } = await import(
  pathToFileURL(join(root, 'crates/kchess-node/dist/index.js')).href
)

function host(dir) {
  return {
    dataDir: dir,
    managedEngineDir: join(dir, 'engines'),
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
  }
}

const samples = []
for (let run = 0; run < runs; run++) {
  const dir = await mkdtemp(join(tmpdir(), 'kchess-network-perf-'))
  await writeFile(join(dir, 'kchess-data.json'), '{}\n')
  await writeFile(join(dir, 'lichess-tokens.json'), '{}\n')
  const core = createKChessCore(host(dir))
  await core.loadData()
  const before = requests
  const start = performance.now()
  const values = await Promise.all(Array.from({ length: calls }, () => core.profile('FixtureUser')))
  const elapsed = performance.now() - start
  samples.push({ elapsedMs: +elapsed.toFixed(1), wireRequests: requests - before })
  if (!values.every((value) => value.title === 'IM')) throw new Error('profile mismatch')
  await core.close()
  await rm(dir, { recursive: true, force: true })
}

const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]
const output = {
  label,
  calls,
  fixtureDelayMs: delay,
  runs: samples,
  medianMs: median(samples.map((run) => run.elapsedMs)),
  medianWireRequests: median(samples.map((run) => run.wireRequests)),
}
console.log(JSON.stringify(output, null, 2))
const outDir = process.env.PERF_OUT ? resolve(process.env.PERF_OUT) : resolve(root, 'perf/results')
await mkdir(outDir, { recursive: true })
await writeFile(resolve(outDir, `network-${label}.json`), `${JSON.stringify(output, null, 2)}\n`)
server.close()
