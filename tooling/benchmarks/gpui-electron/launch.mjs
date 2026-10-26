// Paired launch benchmark of the packaged Electron and GPUI apps, measured from outside the process
// so both are timed the same way (docs/benchmarks/2026-10-26-gpui-vs-electron.md).
// Build first: `pnpm run pack:mac` and `node tooling/gpui-package.mjs`.
// Usage: node tooling/benchmarks/gpui-electron/launch.mjs [pairs=7] [out.json]
import { execFileSync, spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(here, '../../..')
const pairs = Number(process.argv[2] ?? 7)
const out = process.argv[3] ?? path.join(repo, 'perf/results/gpui-electron-launch.json')
// The window probe (ScreenCaptureKit), compiled once beside the other perf results.
const probe = path.join(repo, 'perf/results/pshot')
if (
  !existsSync(probe) ||
  statSync(probe).mtimeMs < statSync(path.join(here, 'pshot.swift')).mtimeMs
) {
  execFileSync('mkdir', ['-p', path.dirname(probe)])
  execFileSync('swiftc', ['-O', '-parse-as-library', '-o', probe, path.join(here, 'pshot.swift')], {
    stdio: 'inherit',
  })
}
const IDLE_FROM_S = 15
const IDLE_WINDOW_S = 30

const apps = {
  electron: path.join(repo, 'dist/mac-arm64/KChess.app/Contents/MacOS/KChess'),
  gpui: path.join(repo, 'target/gpui-package/stage/KChess Preview.app/Contents/MacOS/kchess'),
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const nowMs = () => performance.now()

function psRows() {
  const text = execFileSync('ps', ['-axo', 'pid=,ppid=,rss=,time=,comm='], {
    encoding: 'utf8',
    maxBuffer: 16 << 20,
  })
  const rows = []
  for (const line of text.split('\n')) {
    const m = line.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+(\S+)\s+(.*)$/)
    if (!m) continue
    rows.push({ pid: +m[1], ppid: +m[2], rssKiB: +m[3], cpuS: cpuSeconds(m[4]), comm: m[5] })
  }
  return rows
}

// ps time: [[dd-]hh:]mm:ss.cc
function cpuSeconds(t) {
  let days = 0
  if (t.includes('-')) [days, t] = [Number(t.split('-')[0]), t.split('-')[1]]
  const parts = t.split(':').map(Number)
  let s = 0
  for (const p of parts) s = s * 60 + p
  return days * 86400 + s
}

function tree(rootPid) {
  const rows = psRows()
  const byParent = new Map()
  for (const r of rows) byParent.set(r.ppid, [...(byParent.get(r.ppid) ?? []), r])
  const result = rows.filter((r) => r.pid === rootPid)
  const visit = (pid) => {
    for (const c of byParent.get(pid) ?? []) {
      result.push(c)
      visit(c.pid)
    }
  }
  visit(rootPid)
  return result
}

function role(row, rootPid) {
  const c = row.comm.toLowerCase()
  if (row.pid === rootPid) return 'main'
  if (c.includes('stockfish')) return 'engine'
  if (c.includes('(renderer)')) return 'renderer'
  if (c.includes('(gpu)')) return 'gpu'
  if (c.includes('helper')) return 'helper'
  return 'other'
}

function footprintKiB(pid) {
  const r = spawnSync('footprint', ['-p', String(pid)], { encoding: 'utf8' })
  const m = (r.stdout ?? '').match(/phys_footprint:\s*([\d.]+)\s*([KMG])B/i)
  if (!m) return null
  return Math.round(Number(m[1]) * { K: 1, M: 1024, G: 1024 * 1024 }[m[2].toUpperCase()])
}

function threads(pid) {
  const r = spawnSync('ps', ['-M', '-p', String(pid)], { encoding: 'utf8' })
  return Math.max(0, (r.stdout ?? '').trim().split('\n').length - 1)
}

function energy(pids) {
  // top's POWER column (Activity Monitor's Energy Impact); the last of three 5 s samples.
  const args = ['-l', '3', '-s', '5', '-stats', 'pid,power']
  for (const p of pids) args.push('-pid', String(p))
  const r = spawnSync('top', args, { encoding: 'utf8' })
  const blocks = (r.stdout ?? '').split(/^PID\s+POWER\s*$/m)
  const last = blocks.at(-1) ?? ''
  let sum = 0
  for (const line of last.split('\n')) {
    const m = line.trim().match(/^(\d+)\s+([\d.]+)/)
    if (m && pids.includes(+m[1])) sum += Number(m[2])
  }
  return Math.round(sum * 10) / 10
}

async function waitGone(rootPid, timeoutMs) {
  const start = nowMs()
  while (nowMs() - start < timeoutMs) {
    if (tree(rootPid).length === 0) return Math.round(nowMs() - start)
    await sleep(25)
  }
  return null
}

async function launch(kind, profile, label) {
  const env = {
    ...process.env,
    KCHESS_USER_DATA_DIR: profile,
  }
  const probe = spawnSync(probe, [apps[kind], '5000'], {
    env,
    encoding: 'utf8',
  })
  const started = nowMs()
  const p = JSON.parse(probe.stdout.trim().split('\n').at(-1))
  const pid = p.pid
  // CPU settle: first 500 ms window after content where the tree used under 5% of one core.
  const samples = []
  let settledMs = null
  while (nowMs() - started < IDLE_FROM_S * 1000) {
    const rows = tree(pid)
    samples.push({ t: nowMs() - started, cpu: rows.reduce((s, r) => s + r.cpuS, 0) })
    for (let i = samples.length - 1; i >= 0 && settledMs === null; i--) {
      const dt = (samples.at(-1).t - samples[i].t) / 1000
      if (dt >= 0.5) {
        if ((samples.at(-1).cpu - samples[i].cpu) / dt < 0.05) settledMs = samples[i].t
        break
      }
    }
    if (settledMs !== null) break
    await sleep(100)
  }
  // Sampling starts when the probe exits, after its capture window.
  const baseMs = (p.firstFrameMs ?? 0) + 5000
  await sleep(Math.max(0, IDLE_FROM_S * 1000 - (nowMs() - started)))
  const rows = tree(pid)
  const procs = rows.map((r) => ({
    pid: r.pid,
    role: role(r, pid),
    comm: r.comm.split('/').at(-1),
    rssKiB: r.rssKiB,
    footprintKiB: footprintKiB(r.pid),
    threads: threads(r.pid),
  }))
  const cpu0 = rows.reduce((s, r) => s + r.cpuS, 0)
  const t0 = nowMs()
  const energyImpact = energy(rows.map((r) => r.pid))
  await sleep(Math.max(0, IDLE_WINDOW_S * 1000 - (nowMs() - t0)))
  const rows1 = tree(pid)
  const cpu1 = rows1.reduce((s, r) => s + r.cpuS, 0)
  const idleCpuPct = ((cpu1 - cpu0) / ((nowMs() - t0) / 1000)) * 100
  const startupCpuS = cpu0
  process.kill(pid, 'SIGTERM')
  let exitMs = await waitGone(pid, 8000)
  if (exitMs === null) {
    for (const r of tree(pid)) process.kill(r.pid, 'SIGKILL')
    await waitGone(pid, 3000)
  }
  // Stragglers reparented to launchd (e.g. an engine) would be missed by tree(); check by comm.
  const result = {
    kind,
    label,
    probe: p,
    // CPU settle is measured from launch; reported relative to launch.
    settledMs: settledMs === null ? null : Math.round(settledMs + baseMs),
    contentBaseMs: baseMs,
    processes: procs,
    totals: {
      processCount: procs.length,
      rssKiB: procs.reduce((s, r) => s + r.rssKiB, 0),
      footprintKiB: procs.reduce((s, r) => s + (r.footprintKiB ?? 0), 0),
      threads: procs.reduce((s, r) => s + r.threads, 0),
    },
    startupCpuS: Math.round(startupCpuS * 100) / 100,
    idleCpuPct: Math.round(idleCpuPct * 100) / 100,
    energyImpact,
    exitAfterSigtermMs: exitMs,
  }
  console.log(
    `${label} ${kind}: window ${p.windowMs} paint ${p.firstPaintMs} complete ${p.completeMs} settled ${result.settledMs} rss ${(result.totals.rssKiB / 1024).toFixed(0)}M fp ${(result.totals.footprintKiB / 1024).toFixed(0)}M idle ${result.idleCpuPct}% energy ${energyImpact} exit ${exitMs}`,
  )
  return result
}

const results = {
  startedAt: new Date().toISOString(),
  host: {
    release: os.release(),
    cpu: os.cpus()[0]?.model,
    cpus: os.cpus().length,
    memGiB: os.totalmem() / 2 ** 30,
  },
  apps,
  launches: [],
}
// Warm-up: one untimed launch each so the binaries are in the file cache.
for (const kind of ['electron', 'gpui']) {
  const dir = mkdtempSync(path.join(os.tmpdir(), `kbench-warm-${kind}-`))
  await launch(kind, dir, 'warmup')
  rmSync(dir, { recursive: true, force: true })
}
const reused = {}
for (const kind of Object.keys(apps))
  reused[kind] = mkdtempSync(path.join(os.tmpdir(), `kbench-reused-${kind}-`))
for (const mode of ['fresh', 'reused']) {
  for (let i = 0; i < pairs; i++) {
    const order = i % 2 === 0 ? ['electron', 'gpui'] : ['gpui', 'electron']
    for (const kind of order) {
      const dir =
        mode === 'fresh' ? mkdtempSync(path.join(os.tmpdir(), `kbench-${kind}-`)) : reused[kind]
      // The reused profile is created by its first launch, which is not counted.
      if (mode === 'reused' && i === 0 && kind === order[0]) {
        for (const k of order) await launch(k, reused[k], 'reused-seed')
      }
      results.launches.push({ mode, pair: i, ...(await launch(kind, dir, `${mode}#${i}`)) })
      if (mode === 'fresh') rmSync(dir, { recursive: true, force: true })
      writeFileSync(out, JSON.stringify(results, null, 1))
    }
  }
}
for (const dir of Object.values(reused)) rmSync(dir, { recursive: true, force: true })
results.finishedAt = new Date().toISOString()
writeFileSync(out, JSON.stringify(results, null, 1))
console.log('done', out)
