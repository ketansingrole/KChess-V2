// Medians and IQRs per app and profile mode from launch.mjs output.
// Usage: node tooling/benchmarks/gpui-electron/summary.mjs <launch.json>
import { readFileSync } from 'node:fs'

const r = JSON.parse(readFileSync(process.argv[2], 'utf8'))
const q = (xs, p) => {
  const s = xs.filter((x) => x !== null && x !== undefined).sort((a, b) => a - b)
  if (!s.length) return null
  const i = (s.length - 1) * p
  return s[Math.floor(i)] + (s[Math.ceil(i)] - s[Math.floor(i)]) * (i - Math.floor(i))
}
const metrics = {
  windowMs: (l) => l.probe.windowMs,
  firstPaintMs: (l) => l.probe.firstPaintMs,
  completeMs: (l) => l.probe.completeMs,
  startupCpuS: (l) => l.startupCpuS,
  rssMiB: (l) => l.totals.rssKiB / 1024,
  footprintMiB: (l) => l.totals.footprintKiB / 1024,
  processes: (l) => l.totals.processCount,
  threads: (l) => l.totals.threads,
  idleCpuPct: (l) => l.idleCpuPct,
  energyImpact: (l) => l.energyImpact,
  exitMs: (l) => l.exitAfterSigtermMs,
}
const summary = {}
for (const mode of ['fresh', 'reused']) {
  for (const kind of ['electron', 'gpui']) {
    const ls = r.launches.filter((l) => l.mode === mode && l.kind === kind)
    const row = { n: ls.length }
    for (const [name, f] of Object.entries(metrics)) {
      const xs = ls.map(f)
      row[name] = {
        p50: q(xs, 0.5),
        p25: q(xs, 0.25),
        p75: q(xs, 0.75),
        min: q(xs, 0),
        max: q(xs, 1),
      }
    }
    summary[`${mode}/${kind}`] = row
  }
}
// Per-role memory breakdown (reused profile).
const roles = {}
for (const kind of ['electron', 'gpui']) {
  const ls = r.launches.filter((l) => l.mode === 'reused' && l.kind === kind)
  const names = new Set(ls.flatMap((l) => l.processes.map((p) => p.role)))
  roles[kind] = {}
  for (const role of names) {
    const per = (field) =>
      q(
        ls.map(
          (l) =>
            l.processes.filter((p) => p.role === role).reduce((s, p) => s + (p[field] ?? 0), 0) /
            1024,
        ),
        0.5,
      )
    roles[kind][role] = { rssMiB: per('rssKiB'), footprintMiB: per('footprintKiB') }
  }
}
const fmt = (v) => (v === null ? '—' : Math.abs(v) >= 100 ? v.toFixed(0) : v.toFixed(2))
for (const [key, row] of Object.entries(summary)) {
  console.log(`\n${key} (n=${row.n})`)
  for (const name of Object.keys(metrics)) {
    const m = row[name]
    console.log(
      `  ${name.padEnd(13)} p50 ${fmt(m.p50).padStart(8)}  IQR ${fmt(m.p25)}–${fmt(m.p75)}  range ${fmt(m.min)}–${fmt(m.max)}`,
    )
  }
}
console.log('\nroles (reused, p50 MiB):', JSON.stringify(roles, null, 1))
