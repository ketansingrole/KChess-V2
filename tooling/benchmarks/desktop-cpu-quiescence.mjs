#!/usr/bin/env node
// Observe Stockfish process CPU time while Analysis is active and after cancellation.
import { _electron } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const baselineRoot = '/private/tmp/kchess-benchmark-ts'
const outputPath =
  process.env.KCHESS_CPU_OUTPUT ??
  path.join(root, 'docs/benchmarks/raw/desktop-cpu-quiescence.json')
if (!process.env.ELECTRON_EXEC_PATH)
  throw new Error('Use tooling/with-branded-electron.mjs as the Playwright launcher.')

const roots = [root, baselineRoot]
const variants = roots.map((repoRoot, index) => {
  const executablePath = path.join(repoRoot, 'dist/mac-arm64/KChess.app/Contents/MacOS/KChess')
  return {
    id: index === 0 ? 'rust' : 'typescript',
    root: repoRoot,
    revision:
      index === 0
        ? (process.env.KCHESS_BENCHMARK_REVISION ??
          execFileSync('git', ['-C', repoRoot, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim())
        : 'f479f7c',
    executablePath,
    executableSha256: execFileSync('shasum', ['-a', '256', executablePath], { encoding: 'utf8' })
      .trim()
      .split(/\s+/)[0],
  }
})

function cpuSeconds(value) {
  const daySplit = value.split('-')
  const days = daySplit.length === 2 ? Number(daySplit[0]) : 0
  const clock = daySplit.at(-1).split(':').map(Number)
  if (clock.some(Number.isNaN)) return null
  if (clock.length === 3) return days * 86_400 + clock[0] * 3600 + clock[1] * 60 + clock[2]
  if (clock.length === 2) return days * 86_400 + clock[0] * 60 + clock[1]
  if (clock.length === 1) return days * 86_400 + clock[0]
  return null
}

function processSnapshot(rootPid) {
  const output = execFileSync('ps', ['-axo', 'pid=,ppid=,rss=,time=,%cpu=,command='], {
    encoding: 'utf8',
    maxBuffer: 8 * 1024 * 1024,
  })
  const rows = []
  for (const line of output.split('\n')) {
    const match = line.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+(\S+)\s+([\d.]+)\s+(.*)$/)
    if (!match) continue
    rows.push({
      pid: Number(match[1]),
      ppid: Number(match[2]),
      rssKiB: Number(match[3]),
      cumulativeCpuTime: match[4],
      cumulativeCpuSeconds: cpuSeconds(match[4]),
      psReportedCpuPercent: Number(match[5]),
      command: match[6].slice(0, 500),
    })
  }
  const children = new Map()
  for (const row of rows) {
    const list = children.get(row.ppid) ?? []
    list.push(row)
    children.set(row.ppid, list)
  }
  const tree = []
  const visit = (pid) => {
    for (const child of children.get(pid) ?? []) {
      tree.push(child)
      visit(child.pid)
    }
  }
  const rootRow = rows.find((row) => row.pid === rootPid)
  if (rootRow) tree.unshift(rootRow)
  visit(rootPid)
  const engine = tree.filter((row) => /stockfish/i.test(row.command))
  return {
    rootPid,
    engineProcessCount: engine.length,
    engineRssKiB: engine.reduce((sum, row) => sum + row.rssKiB, 0),
    engineCpuSeconds: engine.reduce((sum, row) => sum + (row.cumulativeCpuSeconds ?? 0), 0),
    engineProcesses: engine,
  }
}

const results = {
  schemaVersion: 1,
  startedAt: new Date().toISOString(),
  host: {
    platform: process.platform,
    release: os.release(),
    architecture: os.arch(),
    cpuModel: os.cpus()[0]?.model ?? null,
    logicalCpuCount: os.cpus().length,
    nodeVersion: process.version,
  },
  methodology: {
    app: 'Packaged unsigned macOS arm64 builds launched by Playwright through tooling/with-branded-electron.mjs; each launch uses a fresh temporary profile, empty KCHESS_NUXT_URL, and no connected account.',
    workload:
      'Open Analysis board, enable Infinite analysis in Engine settings, wait for a depth update without the finite /26 suffix, sample its descendant process tree every 1 s for 5 samples, switch Engine analysis off, wait for Engine off, then sample every 1 s for 10 samples.',
    analysis:
      'Infinite analysis with the packaged defaults (Hash 128 MB, MultiPV 3); settings are not modified beyond enabling the explicit infinite-analysis switch.',
    cpu: 'The engine process cumulative CPU clock is read from macOS ps time every sample. A flat cumulative time after cancellation indicates the process remains resident without consuming additional CPU. ps %cpu is retained only as a decaying average over up to one minute per local ps(1); RSS in KiB can double-count shared pages.',
    order:
      'Three paired runs, serial launches, counterbalanced order: Rust then TypeScript in pair 1 and 3; TypeScript then Rust in pair 2.',
    limit:
      'This is a small cancellation/quiescence check, not a replacement for the 7-pair startup study. An engine process remaining allocated after stop can be a warm reusable engine and does not alone establish a memory leak.',
  },
  variants,
  runs: [],
}

async function launchAndSample(variant, pair, order) {
  const profile = await mkdtemp(path.join(os.tmpdir(), `kchess-cpu-${variant.id}-`))
  const record = {
    variant: variant.id,
    revision: variant.revision,
    pair,
    order,
    observations: [],
    profile: 'fresh temporary profile',
    errors: [],
  }
  let app
  let page
  try {
    app = await _electron.launch({
      executablePath: variant.executablePath,
      args: [],
      cwd: variant.root,
      timeout: 60_000,
      env: {
        ...process.env,
        KCHESS_NUXT_URL: '',
        KCHESS_USER_DATA_DIR: profile,
      },
    })
    record.onBatteryPower = await app
      .evaluate(({ powerMonitor }) => powerMonitor.isOnBatteryPower())
      .catch((error) => {
        record.onBatteryPowerError = String(error?.message ?? error)
        return null
      })
    page = await app.firstWindow()
    await page.locator('section[aria-label="Start"]').waitFor({ state: 'visible', timeout: 45_000 })
    await page.locator('aside').getByText('Analysis board', { exact: true }).click()
    await page
      .getByRole('heading', { name: 'Analysis board' })
      .waitFor({ state: 'visible', timeout: 15_000 })
    const status = page.locator('.ceval-status')
    const toggle = page.getByRole('switch', { name: 'Engine analysis' })
    if (!(await toggle.isEnabled())) {
      record.engineStatus = (await status.textContent())?.trim() ?? 'disabled'
      record.workload = 'engine unavailable'
      return record
    }
    await page.waitForFunction(
      () => /Depth \d+/.test(document.querySelector('.ceval-status')?.textContent ?? ''),
      null,
      { timeout: 10_000 },
    )
    await page.getByRole('button', { name: 'Engine settings', exact: true }).click()
    await page.getByRole('switch', { name: 'Infinite analysis' }).check()
    await page.keyboard.press('Escape')
    await page.waitForFunction(
      () =>
        /Depth \d+/.test(document.querySelector('.ceval-status')?.textContent ?? '') &&
        !/\/\d+/.test(document.querySelector('.ceval-status')?.textContent ?? ''),
      null,
      { timeout: 15_000 },
    )
    record.engineMode = 'infinite-analysis'
    record.engineStatusAtStart = (await status.textContent())?.trim()
    for (let index = 0; index < 5; index += 1) {
      record.observations.push({
        phase: 'active',
        sample: index + 1,
        at: new Date().toISOString(),
        ...processSnapshot(app.process().pid),
      })
      if (index < 4) await new Promise((resolve) => setTimeout(resolve, 1000))
    }
    const stopStarted = performance.now()
    await toggle.click()
    await page.waitForFunction(
      () => document.querySelector('.ceval-status')?.textContent?.trim() === 'Engine off',
      null,
      { timeout: 10_000 },
    )
    record.cancelLatencyMs = Number((performance.now() - stopStarted).toFixed(2))
    for (let index = 0; index < 10; index += 1) {
      record.observations.push({
        phase: 'cancelled',
        sample: index + 1,
        at: new Date().toISOString(),
        ...processSnapshot(app.process().pid),
      })
      if (index < 9) await new Promise((resolve) => setTimeout(resolve, 1000))
    }
    record.workload = 'engine started and cancelled'
    record.success = true
  } catch (error) {
    record.success = false
    record.errors.push(String(error?.stack ?? error))
  } finally {
    if (app) {
      try {
        if (page) {
          const toggle = page.getByRole('switch', { name: 'Engine analysis' })
          if (await toggle.isChecked()) {
            await toggle.click()
            await page.waitForFunction(
              () => document.querySelector('.ceval-status')?.textContent?.trim() === 'Engine off',
              null,
              { timeout: 10_000 },
            )
          }
        }
        await app.close()
      } catch (error) {
        record.errors.push(`close/cancel: ${String(error?.stack ?? error)}`)
      }
    }
    await rm(profile, { recursive: true, force: true })
    record.finishedAt = new Date().toISOString()
    results.runs.push(record)
    await mkdir(path.dirname(outputPath), { recursive: true })
    results.completedAt = new Date().toISOString()
    await writeFile(outputPath, `${JSON.stringify(results, null, 2)}\n`)
    process.stdout.write(
      `${JSON.stringify({ variant: record.variant, pair, workload: record.workload, cancelLatencyMs: record.cancelLatencyMs })}\n`,
    )
  }
  return record
}

for (let pair = 1; pair <= 3; pair += 1) {
  const order = pair % 2 ? variants : [...variants].reverse()
  for (const variant of order)
    await launchAndSample(
      variant,
      pair,
      order.map((item) => item.id),
    )
}

if (results.runs.some((run) => !run.success && run.workload !== 'engine unavailable'))
  process.exitCode = 1
