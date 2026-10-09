// The built core (core/dist) through its public API: a profile seeded with the benchmark
// fixtures, timed on `library()` (what every frontend loads at startup) and on a study
// command (which re-reads the study library). Run by tooling/core-benchmark.mjs.
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('..', import.meta.url))
const [fixtures, out] = process.argv.slice(2)
if (!fixtures || !out) throw new Error('Usage: core-library-benchmark.mjs <fixtures> <out.json>')
const { createKChessCore, BUNDLED_ENGINE_SCRIPT, performanceSnapshot } = await import(
  join(root, 'core/dist/index.js')
)

const dataDir = await mkdtemp(join(tmpdir(), 'kchess-core-benchmark-'))
const platform = {
  dataDir,
  bundledEnginePath: join(root, BUNDLED_ENGINE_SCRIPT),
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
}
const core = createKChessCore(platform)

const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]
async function time(fn, runs) {
  const values = []
  for (let i = 0; i < runs; i++) {
    const start = performance.now()
    await fn()
    values.push(performance.now() - start)
  }
  return +median(values).toFixed(2)
}

try {
  const read = (name) => readFile(join(fixtures, name), 'utf8')
  const games = JSON.parse(await read('games.json'))
  await core.importLibrary({
    'kchess:studies:v1': await read('studies.json'),
    'kchess:game-history:v1': JSON.stringify({ version: 1, games }),
    'kchess:mistakes:v1': await read('mistakes.json'),
  })
  // A fresh core over the seeded profile: the first library() is the startup load.
  await core.close()
  const fresh = createKChessCore(platform)
  const start = performance.now()
  const snapshot = await fresh.library()
  const firstMs = +(performance.now() - start).toFixed(2)
  const study = snapshot.studies[0]
  const result = {
    rules: performanceSnapshot().rules,
    counts: {
      games: snapshot.games.length,
      studies: snapshot.studies.length,
      mistakes: snapshot.mistakes.length,
    },
    libraryFirstMs: firstMs,
    libraryMs: await time(() => fresh.library(), 15),
    studyRenameMs: await time(
      () => fresh.studyCommand({ op: 'rename', id: study.id, name: `Renamed ${Math.random()}` }),
      15,
    ),
  }
  await fresh.close()
  await writeFile(out, JSON.stringify(result, null, 2) + '\n')
  console.info(`[benchmark] core API (${result.rules}): ${JSON.stringify(result)}`)
} finally {
  await rm(dataDir, { recursive: true, force: true })
}
