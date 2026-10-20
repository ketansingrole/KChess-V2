// Library snapshot benchmark: studies, archived games, sessions.
// Usage: node perf/bench-library.mjs [--label=before] [--studies=20] [--chapters=20] [--games=200]
import { mkdir, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { performance } from 'node:perf_hooks'
import { createKChessCore } from '../crates/kchess-node/dist/index.js'
import { nativeRules } from '../crates/kchess-node/dist/native.js'

const args = Object.fromEntries(
  process.argv
    .slice(2)
    .filter((a) => a.startsWith('--'))
    .map((a) => {
      const [k, v] = a.replace(/^--/, '').split('=')
      return [k, v ?? 'true']
    }),
)
const label = args.label ?? 'run'
const studyCount = Number(args.studies ?? 20)
const chapterCount = Number(args.chapters ?? 20)
const gameCount = Number(args.games ?? 200)
const plies = Number(args.plies ?? 80)
const profileRoot = join(tmpdir(), `kchess-perf-library-${process.pid}`)

const native = nativeRules()
const invoke = (method, ...a) => JSON.parse(native.invoke(method, JSON.stringify(a)))
const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const moves = []
const sans = []
{
  let fen = START
  let seed = 999
  for (let i = 0; i < plies; i++) {
    const legal = invoke('legalMoves', { variant: 'standard', fen })
    if (!legal.length) break
    seed = (seed * 1103515245 + 12345) % 2147483648
    const p = legal[seed % legal.length]
    const promotion = p.promotion
      ? { queen: 'q', rook: 'r', bishop: 'b', knight: 'n' }[p.promotion]
      : ''
    const uci = `${p.from}${p.to}${promotion}`
    const played = invoke('play', { variant: 'standard', fen }, uci)
    moves.push(uci)
    sans.push(played.san)
    fen = played.position.fen
  }
}
const pgn = invoke('pgnFromSan', sans)
const pgnB = invoke('pgnFromSan', sans.slice(0, Math.max(2, plies - 2)))

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

await mkdir(profileRoot, { recursive: true })
const dataDir = join(profileRoot, 'profile')
await mkdir(dataDir, { recursive: true })
await writeFile(join(dataDir, 'kchess-data.json'), '{}\n')
await writeFile(join(dataDir, 'lichess-tokens.json'), '{}\n')
const seedStart = performance.now()
const core = createKChessCore(host(dataDir))
await core.loadData()
for (let s = 0; s < studyCount; s++) {
  const chapters = Array.from({ length: chapterCount }, (_, c) => ({
    name: `Chapter ${c + 1}`,
    pgn: c % 3 === 0 ? pgn : pgnB,
  }))
  await core.studyCommand({ op: 'saveChapters', name: `Study ${s + 1}`, chapters })
}
for (let g = 0; g < gameCount; g++) {
  await core.saveArchivedGame({
    id: `game-${g}`,
    source: 'computer',
    startedAt: 1_700_000_000_000 + g,
    updatedAt: 1_700_000_100_000 + g,
    white: 'Player',
    black: 'Rival',
    result: g % 2 ? '1-0' : '0-1',
    reason: 'mate',
    finished: true,
    setup: { variant: 'standard', fen: START },
    moves,
    timeControl: '5+0',
  })
}
const seedMs = performance.now() - seedStart
await core.close()

const samples = {}
async function measure(name, work, n = 5) {
  let last
  for (let i = 0; i < n; i++) {
    const start = performance.now()
    last = await work()
    ;(samples[name] ??= []).push(performance.now() - start)
  }
  return last
}

const openStart = performance.now()
const core2 = createKChessCore(host(dataDir))
await core2.loadData()
const startupMs = performance.now() - openStart
let snapshot
try {
  snapshot = await measure('library_first', () => core2.library(), 1)
  const size = JSON.stringify(snapshot).length
  for (let i = 0; i < 5; i++) await measure('library_warm', () => core2.library(), 1)
  await measure('studySavePgn', () =>
    core2.studyCommand(
      { op: 'save', id: snapshot.studies[0].id, name: 'Study 1', pgn: pgnB },
      true,
    ),
  )
  await measure('saveArchivedGame', () =>
    core2.saveArchivedGame({
      id: 'game-0',
      source: 'computer',
      startedAt: 1_700_000_000_000,
      updatedAt: 1_700_000_200_000,
      white: 'Player',
      black: 'Rival',
      result: '1-0',
      reason: 'mate',
      finished: true,
      setup: { variant: 'standard', fen: START },
      moves,
      timeControl: '5+0',
    }),
  )
  const result = {
    label,
    studyCount,
    chapterCount,
    gameCount,
    plies,
    seedMs: +seedMs.toFixed(1),
    startupMs: +startupMs.toFixed(1),
    snapshotBytes: size,
    samples: Object.fromEntries(
      Object.entries(samples).map(([key, values]) => {
        const sorted = [...values].sort((a, b) => a - b)
        return [
          key,
          {
            n: sorted.length,
            p50: +sorted[Math.floor(sorted.length / 2)].toFixed(2),
            min: +sorted[0].toFixed(2),
          },
        ]
      }),
    ),
  }
  console.log(JSON.stringify(result, null, 2))
  const outDir = process.env.PERF_OUT ? resolve(process.env.PERF_OUT) : resolve('perf/results')
  await mkdir(outDir, { recursive: true })
  await writeFile(join(outDir, `library-${label}.json`), `${JSON.stringify(result, null, 2)}\n`)
} finally {
  await core2.close()
  await rm(profileRoot, { recursive: true, force: true })
}
