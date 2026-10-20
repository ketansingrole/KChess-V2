// Standalone core benchmark: seeds local SQLite fixtures, then times warm core API calls.
// Usage: node perf/bench-core.mjs [--games=50000] [--opening-labels=2] [--runs=12] [--label=before]
import { DatabaseSync } from 'node:sqlite'
import { mkdir, writeFile, rm } from 'node:fs/promises'
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
const { createKChessCore } = await import(
  pathToFileURL(join(root, 'crates/kchess-node/dist/index.js')).href
)
const games = Number(args.games ?? 50_000)
const openingLabels = Number(args['opening-labels'] ?? 2)
const runs = Number(args.runs ?? 12)
const label = args.label ?? 'run'
const profileRoot = resolve(args.dir ?? join(tmpdir(), `kchess-perf-core-${process.pid}`))

const OPENINGS = [
  'Italian Game',
  "Queen's Pawn Game",
  'Sicilian Defense: Najdorf',
  'Sicilian Defense: Dragon',
  'French Defense: Winawer',
  'Ruy Lopez: Morphy Defense',
  'King\u2019s Indian Defense',
  'Caro-Kann Defense: Advance',
  'English Opening: Symmetrical',
  'Dutch Defense: Stonewall',
  'Scandinavian Defense',
  'Alekhine Defense',
  'Vienna Game',
  'London System',
  'Catalan Opening',
  'Pirc Defense',
  'Modern Defense',
  'Benoni Defense',
  'Bird Opening',
  'Nimzo-Indian Defense',
]

const pgn = `[Event "Benchmark"]\n[Site "local"]\n[Date "2026.01.01"]\n[White "Player"]\n[Black "Opponent"]\n[Result "*"]\n\n1. e4 e5 2. Nf3 Nc6 3. Bc4 *`

function seed(dir) {
  const db = new DatabaseSync(join(dir, 'kchess.db'))
  try {
    db.prepare(
      'INSERT OR REPLACE INTO accounts (username, connected, lastSyncedAt) VALUES (?, 0, NULL)',
    ).run('benchmark')
    const insert = db.prepare(`INSERT INTO games
      (account, id, createdAt, lastMoveAt, rated, speed, perf, status, winner, color,
       opponent, opponentRating, playerRating, ratingDiff, opening, moves, pgn)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`)
    db.exec('BEGIN IMMEDIATE')
    try {
      for (let index = 0; index < games; index++) {
        const date = 1_700_000_000_000 + index * 60_000
        const outcome = index % 3
        insert.run(
          'benchmark',
          `bench-${String(index).padStart(7, '0')}`,
          date,
          date + 180_000,
          index % 2,
          index % 4 === 0 ? 'rapid' : 'blitz',
          index % 4 === 0 ? 'rapid' : 'blitz',
          'mate',
          outcome === 2 ? null : outcome === 0 ? 'white' : 'black',
          index % 2 === 0 ? 'white' : 'black',
          `Opponent ${index % 1000}`,
          1200 + (index % 1200),
          1250 + (index % 1000),
          (index % 41) - 20,
          openingLabels <= OPENINGS.length
            ? OPENINGS[index % openingLabels]
            : `Opening family ${index % openingLabels}`,
          'e2e4 e7e5 g1f3 b8c6 f1c4',
          pgn,
        )
      }
      db.exec('COMMIT')
    } catch (error) {
      db.exec('ROLLBACK')
      throw error
    }
  } finally {
    db.close()
  }
}

function host(dir) {
  return {
    dataDir: dir,
    bundledEnginePath: undefined,
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

const samples = {}
async function measure(name, work, repeats = 1) {
  let last
  for (let i = 0; i < repeats; i++) {
    const start = performance.now()
    last = await work()
    ;(samples[name] ??= []).push(performance.now() - start)
  }
  return last
}

function percentile(values, p) {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.min(sorted.length - 1, Math.ceil(p * sorted.length) - 1)]
}

await mkdir(profileRoot, { recursive: true })
const dataDir = join(profileRoot, 'profile')
await mkdir(dataDir, { recursive: true })
await writeFile(join(dataDir, 'kchess-data.json'), '{}\n')
await writeFile(join(dataDir, 'lichess-tokens.json'), '{}\n')
const core = createKChessCore(host(dataDir))
await core.loadData()
await core.close()
seed(dataDir)

const moduleStart = performance.now()
const core2 = createKChessCore(host(dataDir))
const data = await core2.loadData()
const startupMs = performance.now() - moduleStart

try {
  const page = await measure('gamePage_cold', () =>
    core2.gamePage({ account: 'benchmark', offset: 0, limit: 100 }),
  )
  await measure('overview_cold', () => core2.gameLibraryOverview())
  await measure('insights_cold', () => core2.insights({ account: 'benchmark', days: 3650 }))
  await measure('library_cold', () => core2.library())

  for (let i = 0; i < runs; i++) {
    await measure('gamePage', () =>
      core2.gamePage({ account: 'benchmark', offset: (i % 5) * 100, limit: 100 }),
    )
    await measure('gamePage_filtered', () =>
      core2.gamePage({ account: 'benchmark', result: 'win', rated: true, offset: 0, limit: 100 }),
    )
    await measure('overview', () => core2.gameLibraryOverview())
    await measure('insights', () => core2.insights({ account: 'benchmark', days: 3650 }))
    await measure('insights_blitz', () =>
      core2.insights({ account: 'benchmark', speed: 'blitz', days: 365 }),
    )
    await measure('library', () => core2.library())
  }

  console.error('DEBUG samples', Object.keys(samples), samples)
  const output = {
    label,
    games,
    openingLabels,
    startupMs,
    accounts: data.accounts.length,
    samples: Object.fromEntries(
      Object.entries(samples).map(([key, values]) => [
        key,
        {
          n: values.length,
          p50: +percentile(values, 0.5).toFixed(3),
          p95: +percentile(values, 0.95).toFixed(3),
        },
      ]),
    ),
    total: page.total,
  }
  console.log(JSON.stringify(output, null, 2))
  const outDir = process.env.PERF_OUT
    ? resolve(process.env.PERF_OUT)
    : resolve(root, 'perf/results')
  await mkdir(outDir, { recursive: true })
  const outFile = join(outDir, `core-${label}-${games}-${openingLabels}.json`)
  await writeFile(outFile, `${JSON.stringify(output, null, 2)}\n`)
} finally {
  await core2.close()
  await rm(profileRoot, { recursive: true, force: true })
}
