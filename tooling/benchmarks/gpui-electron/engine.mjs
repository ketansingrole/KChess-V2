// The same UCI workload against the engine each packaged app ships
// (docs/benchmarks/2026-10-26-gpui-vs-electron.md).
// Usage: node tooling/benchmarks/gpui-electron/engine.mjs [out.json]
import { execFileSync, spawn } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const electronApp = `${repo}/dist/mac-arm64/KChess.app`
const engines = {
  'electron (Stockfish 19 lite, WASM in Node)': {
    cmd: `${electronApp}/Contents/MacOS/KChess`,
    args: [
      `${electronApp}/Contents/Resources/app.asar.unpacked/node_modules/stockfish/bin/stockfish-19-lite.js`,
    ],
    env: { ELECTRON_RUN_AS_NODE: '1' },
  },
  'gpui (Stockfish 19, native)': {
    cmd: `${repo}/target/gpui-package/stage/KChess Preview.app/Contents/MacOS/stockfish`,
    args: [],
    env: {},
  },
}
const positions = {
  startpos: 'startpos',
  italian: 'fen r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3',
  middlegame: 'fen r2q1rk1/pp2bppp/2n1pn2/2pp4/3P1B2/2PBPN2/PP1N1PPP/R2Q1RK1 w - - 0 9',
  endgame: 'fen 8/5pk1/6p1/3R4/5P2/1r4P1/5K2/8 w - - 0 45',
}
const MOVETIME = 3000

function rssKiB(pid) {
  try {
    return Number(
      execFileSync('ps', ['-o', 'rss=', '-p', String(pid)], { encoding: 'utf8' }).trim(),
    )
  } catch (error) {
    // The engine exited between samples.
    console.warn(`rss sample failed: ${error?.message ?? error}`)
    return 0
  }
}

async function session(engine, threads) {
  const child = spawn(engine.cmd, engine.args, { env: { ...process.env, ...engine.env } })
  let buf = ''
  const waiters = []
  child.stdout.on('data', (d) => {
    buf += d
    let i
    while ((i = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, i).trim()
      buf = buf.slice(i + 1)
      for (const w of [...waiters]) w(line)
    }
  })
  const send = (s) => child.stdin.write(s + '\n')
  const until = (pred) =>
    new Promise((resolve) => {
      const w = (line) => {
        if (pred(line)) {
          waiters.splice(waiters.indexOf(w), 1)
          resolve(line)
        }
      }
      waiters.push(w)
    })
  send('uci')
  await until((l) => l === 'uciok')
  send(`setoption name Threads value ${threads}`)
  send('setoption name Hash value 64')
  send('isready')
  await until((l) => l === 'readyok')
  const out = {}
  let peak = 0
  for (const [name, pos] of Object.entries(positions)) {
    send('ucinewgame')
    send(`position ${pos}`)
    send('isready')
    await until((l) => l === 'readyok')
    let last = null
    const info = (l) => {
      if (l.startsWith('info depth') && l.includes(' nps ')) last = l
    }
    waiters.push(info)
    const sampler = setInterval(() => (peak = Math.max(peak, rssKiB(child.pid))), 200)
    send(`go movetime ${MOVETIME}`)
    await until((l) => l.startsWith('bestmove'))
    clearInterval(sampler)
    waiters.splice(waiters.indexOf(info), 1)
    const num = (k) => Number(last?.match(new RegExp(`\\b${k} (\\d+)`))?.[1] ?? NaN)
    out[name] = { depth: num('depth'), nodes: num('nodes'), nps: num('nps'), timeMs: num('time') }
  }
  send('quit')
  return { positions: out, peakRssMiB: Math.round(peak / 1024) }
}

const results = { movetimeMs: MOVETIME, runs: [] }
for (let rep = 0; rep < 2; rep++) {
  for (const threads of [1, 4]) {
    const names = Object.keys(engines)
    for (const name of rep % 2 ? names.reverse() : names) {
      const r = await session(engines[name], threads)
      results.runs.push({ engine: name, threads, rep, ...r })
      console.log(
        name,
        `threads=${threads}`,
        `rep=${rep}`,
        JSON.stringify(r.positions),
        `peak ${r.peakRssMiB} MiB`,
      )
    }
  }
}
const out = process.argv[2] ?? path.join(repo, 'perf/results/gpui-electron-engine.json')
mkdirSync(path.dirname(out), { recursive: true })
writeFileSync(out, JSON.stringify(results, null, 1))
