#!/usr/bin/env node
// Production Electron startup and renderer workload measurements. Launch through
// tooling/with-branded-electron.mjs so this script always uses the branded binary.
import { _electron } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const currentRoot = path.resolve(scriptDir, '../..')
const roots = process.argv.slice(2).filter((arg) => !arg.startsWith('--'))
const runCountArg = process.argv.find((arg) => arg.startsWith('--runs='))
const runCount = Number(runCountArg?.split('=')[1] ?? 7)
const probeOnly = process.argv.includes('--probe')
const packaged = process.argv.includes('--packaged')
if (!process.env.ELECTRON_EXEC_PATH) {
  throw new Error(
    'Launch with node tooling/with-branded-electron.mjs node tooling/benchmarks/desktop-startup.mjs ...',
  )
}
if (roots.length !== 2 || !Number.isInteger(runCount) || (!probeOnly && runCount < 7)) {
  throw new Error(
    'Usage: desktop-startup.mjs <rust-root> <typescript-baseline-root> [--runs=7+] [--probe]',
  )
}

const variants = roots.map((root, index) => {
  const resolvedRoot = path.resolve(root)
  const revision =
    index === 0
      ? (process.env.KCHESS_BENCHMARK_REVISION ??
        execFileSync('git', ['-C', resolvedRoot, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim())
      : (process.env.KCHESS_BENCHMARK_BASELINE_REVISION ?? 'f479f7c')
  const executablePath = packaged
    ? path.join(resolvedRoot, 'dist/mac-arm64/KChess.app/Contents/MacOS/KChess')
    : process.env.ELECTRON_EXEC_PATH
  const binarySha256 = packaged
    ? execFileSync('shasum', ['-a', '256', executablePath], { encoding: 'utf8' })
        .trim()
        .split(/\s+/)[0]
    : null
  return {
    id:
      index === 0
        ? (process.env.KCHESS_DESKTOP_CURRENT_ID ?? 'rust')
        : (process.env.KCHESS_DESKTOP_BASELINE_ID ?? 'typescript'),
    root: resolvedRoot,
    revision,
    executablePath,
    binarySha256,
  }
})
const outputPath =
  process.env.KCHESS_DESKTOP_OUTPUT ??
  path.join(currentRoot, 'docs/benchmarks/raw/desktop-startup.json')
const writeProbeOutput = probeOnly && Boolean(process.env.KCHESS_DESKTOP_OUTPUT)
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
    electronExecutable: process.env.ELECTRON_EXEC_PATH,
  },
  methodology: {
    launch: `${packaged ? 'Freshly built unsigned macOS arm64 KChess.app package' : 'Production out/ and .output/ build'} launched through Playwright via tooling/with-branded-electron.mjs; KCHESS_NUXT_URL empty; fresh disposable user-data profile per launch; no account connected.`,
    startupTimes:
      'Measured from immediately before Electron launch call to visible section[aria-label="Start"], and then to enabled Play the computer button.',
    idle: 'Wait 2500 ms on Home after startup, then sample Electron app.getAppMetrics().',
    cpu: 'Electron app.getAppMetrics percentCPUUsage is the delta since its previous query and is recorded as returned. macOS ps %cpu is recorded as psReportedCpuPercent; the local ps(1) manual defines it as a decaying average over up to one minute, not lifetime average. Electron memory.workingSetSize and macOS ps rss are in KiB. Per-process RSS sums can double-count shared pages; Electron privateBytes is generally unavailable on macOS. Process-tree group sums label Electron Browser as main, Tab as renderer, GPU as gpu, and command names mentioning Stockfish/engine as engine; remaining descendants are other.',
    workloads:
      'Click Analysis board then Game history through the visible sidebar. Metrics are recorded after each settled page and after returning Home. In Analysis, wait for a Stockfish depth update, sample after 1 s, switch analysis off, wait a fixed 1 s, record whether Stockfish exited, and sample retained process memory. While still off, play e2-e4 to bypass the prior position cache, turn analysis back on, and record time to the first depth update and process memory.',
    parity:
      'Before continuing beyond one launch of each candidate, assert that visible Start region, Play the computer action, Analysis board heading, and Game history heading match.',
    order:
      'Seven paired fresh launches per candidate; odd pairs run current then baseline, even pairs run baseline then current.',
  },
  variants,
  launches: [],
}
let lastMetricQueryAt = null

function processRows(metrics) {
  return metrics.map((metric) => ({
    pid: metric.pid,
    type: metric.type,
    name: metric.name ?? null,
    serviceName: metric.serviceName ?? null,
    memory: metric.memory
      ? {
          workingSetKiB: metric.memory.workingSetSize,
          privateKiB: metric.memory.privateBytes,
        }
      : null,
    cpu: metric.cpu
      ? {
          percentCPUUsage: metric.cpu.percentCPUUsage,
          cumulativeCPUUsage: metric.cpu.cumulativeCPUUsage,
        }
      : null,
  }))
}

function processTreeSnapshot(rootPid, metrics) {
  try {
    const lines = execFileSync('ps', ['-axo', 'pid=,ppid=,rss=,%cpu=,command='], {
      encoding: 'utf8',
      maxBuffer: 8 * 1024 * 1024,
    }).split('\n')
    const rows = []
    for (const line of lines) {
      const match = line.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+([\d.]+)\s+(.*)$/)
      if (!match) continue
      rows.push({
        pid: Number(match[1]),
        ppid: Number(match[2]),
        rssKiB: Number(match[3]),
        psReportedCpuPercent: Number(match[4]),
        command: match[5].slice(0, 500),
      })
    }
    const byParent = new Map()
    for (const row of rows) {
      const siblings = byParent.get(row.ppid) ?? []
      siblings.push(row)
      byParent.set(row.ppid, siblings)
    }
    const descendants = []
    const visit = (pid) => {
      for (const child of byParent.get(pid) ?? []) {
        descendants.push(child)
        visit(child.pid)
      }
    }
    const root = rows.find((row) => row.pid === rootPid)
    if (root) descendants.unshift(root)
    visit(rootPid)
    const electronTypes = new Map(metrics.map((metric) => [metric.pid, metric.type]))
    const grouped = { main: [], renderer: [], gpu: [], engine: [], other: [] }
    for (const row of descendants) {
      const command = row.command.toLowerCase()
      const type = electronTypes.get(row.pid)
      const bucket =
        type === 'Browser'
          ? 'main'
          : type === 'Tab'
            ? 'renderer'
            : type === 'GPU'
              ? 'gpu'
              : /stockfish|engine.*\.js|stockfish.*\.js/.test(command)
                ? 'engine'
                : 'other'
      grouped[bucket].push(row)
    }
    const totals = Object.fromEntries(
      Object.entries(grouped).map(([name, items]) => [
        name,
        {
          processCount: items.length,
          rssKiB: items.reduce((sum, item) => sum + item.rssKiB, 0),
          psReportedCpuPercent: items.reduce((sum, item) => sum + item.psReportedCpuPercent, 0),
        },
      ]),
    )
    return {
      source: 'ps -axo pid,ppid,rss,%cpu,command; descendants of Electron main PID',
      totals,
      processes: descendants,
    }
  } catch (error) {
    return { unavailable: String(error?.message ?? error) }
  }
}

async function captureMetrics(app, label) {
  const captureStart = performance.now()
  const measuredAt = new Date().toISOString()
  const metrics = await app.evaluate(({ app: electronApp }) => electronApp.getAppMetrics())
  const sample = {
    label,
    measuredAt,
    queryElapsedMs: Math.round(performance.now() - captureStart),
    cpuPercentWindowMs:
      lastMetricQueryAt === null ? null : Math.round(captureStart - lastMetricQueryAt),
    processes: processRows(metrics),
    processTree: processTreeSnapshot(app.process().pid, metrics),
  }
  lastMetricQueryAt = performance.now()
  return sample
}

async function runOne(variant, ordinal, runIndex) {
  lastMetricQueryAt = null
  const profile = await mkdtemp(path.join(os.tmpdir(), `kchess-bench-${variant.id}-`))
  const record = {
    variant: variant.id,
    revision: variant.revision,
    ordinal,
    runNumber: runIndex,
    launchStartedAt: new Date().toISOString(),
    profile: 'fresh disposable profile',
    timingsMs: {},
    observations: [],
    errors: [],
  }
  let app
  try {
    const launchStart = performance.now()
    app = await _electron.launch({
      executablePath: variant.executablePath,
      args: packaged ? [] : ['apps/desktop'],
      cwd: variant.root,
      env: {
        ...process.env,
        KCHESS_NUXT_URL: '',
        KCHESS_USER_DATA_DIR: profile,
      },
      timeout: 60_000,
    })
    try {
      record.onBatteryPower = await app.evaluate(({ powerMonitor }) =>
        powerMonitor.isOnBatteryPower(),
      )
    } catch (error) {
      record.onBatteryPowerError = String(error?.message ?? error)
    }
    record.timingsMs.electronLaunchResolved = Math.round(performance.now() - launchStart)
    const page = await app.firstWindow()
    const visibleStart = page.locator('section[aria-label="Start"]')
    await visibleStart.waitFor({ state: 'visible', timeout: 45_000 })
    record.timingsMs.visibleStart = Math.round(performance.now() - launchStart)
    const playButton = page.getByRole('button', { name: 'Play the computer', exact: true })
    await playButton.waitFor({ state: 'visible', timeout: 10_000 })
    await playButton.waitFor({ state: 'attached', timeout: 10_000 })
    await page.waitForFunction(
      () => {
        const button = [...document.querySelectorAll('button')].find((item) =>
          item.textContent?.includes('Play the computer'),
        )
        return Boolean(
          button && !button.disabled && button.getAttribute('aria-disabled') !== 'true',
        )
      },
      null,
      { timeout: 10_000 },
    )
    record.timingsMs.actionablePlayComputer = Math.round(performance.now() - launchStart)
    record.surfaceLabels = {
      startRegion: await visibleStart.getAttribute('aria-label'),
      playComputerAction: (await playButton.innerText()).trim(),
    }

    record.observations.push(await captureMetrics(app, 'startup'))
    await new Promise((resolve) => setTimeout(resolve, 2500))
    record.observations.push(await captureMetrics(app, 'home-idle-2500ms'))

    const analysisNav = page.locator('aside').getByText('Analysis board', { exact: true })
    const analysisStarted = performance.now()
    await analysisNav.click()
    await page
      .getByRole('heading', { name: 'Analysis board' })
      .waitFor({ state: 'visible', timeout: 15_000 })
    record.surfaceLabels.analysisHeading = (
      await page.getByRole('heading', { name: 'Analysis board' }).innerText()
    ).trim()
    record.timingsMs.analysisNavigation = Math.round(performance.now() - analysisStarted)
    record.observations.push(await captureMetrics(app, 'analysis-settled'))
    const engineStatus = page.locator('.ceval-status')
    const engineToggle = page.getByRole('switch', { name: 'Engine analysis' })
    if (await engineToggle.isEnabled()) {
      try {
        await page.waitForFunction(
          () => /Depth \d+/.test(document.querySelector('.ceval-status')?.textContent ?? ''),
          null,
          { timeout: 5_000 },
        )
        record.timingsMs.engineFirstUpdate = Math.round(performance.now() - analysisStarted)
        await new Promise((resolve) => setTimeout(resolve, 1000))
        record.observations.push(await captureMetrics(app, 'analysis-engine-active'))

        await page.getByRole('button', { name: 'Engine settings', exact: true }).click()
        const infiniteSetting = page.getByRole('switch', { name: 'Infinite analysis', exact: true })
        record.analysisEngineSettings = {
          infinite: await infiniteSetting.getAttribute('aria-checked'),
          engineLines: (
            await page.getByRole('combobox', { name: 'Engine lines' }).innerText()
          ).trim(),
        }
        await page.getByRole('button', { name: 'Engine settings', exact: true }).click()

        const cancelStarted = performance.now()
        await engineToggle.click()
        await page.waitForFunction(
          () => document.querySelector('.ceval-status')?.textContent?.trim() === 'Engine off',
          null,
          { timeout: 10_000 },
        )
        record.timingsMs.engineCancellation = Math.round(performance.now() - cancelStarted)
        record.observations.push(await captureMetrics(app, 'analysis-engine-off'))

        const offDeadline = performance.now() + 1000
        let offMetrics
        do {
          offMetrics = await app.evaluate(({ app: electronApp }) => electronApp.getAppMetrics())
          if (
            processTreeSnapshot(app.process().pid, offMetrics).totals?.engine?.processCount === 0
          ) {
            record.engineProcessExitedAfterOff = true
            break
          }
          await new Promise((resolve) => setTimeout(resolve, 100))
        } while (performance.now() < offDeadline)
        if (record.engineProcessExitedAfterOff === undefined) {
          record.engineProcessExitedAfterOff = false
        }
        if (performance.now() < offDeadline) {
          await new Promise((resolve) => setTimeout(resolve, offDeadline - performance.now()))
        }
        record.engineOffMemoryDelayMs = 1000
        record.observations.push(await captureMetrics(app, 'analysis-engine-off-1000ms'))

        const board = page.locator('.analysis-board cg-board')
        const bounds = await board.boundingBox()
        if (!bounds)
          throw new Error('Analysis board is not visible for the cold re-enable position change')
        const moves = page.getByRole('group', { name: 'Moves', exact: true })
        record.movesBeforeEngineReenable = (await moves.innerText()).trim()
        record.boardPiecesBeforeEngineReenable = await page
          .locator('.analysis-board cg-board piece')
          .evaluateAll((pieces) =>
            pieces.map((piece) => `${piece.className} ${piece.getAttribute('style') ?? ''}`).sort(),
          )
        const clickSquare = async (square) => {
          const file = square.charCodeAt(0) - 97
          const rank = Number(square[1]) - 1
          await page.mouse.click(
            bounds.x + ((file + 0.5) * bounds.width) / 8,
            bounds.y + ((7 - rank + 0.5) * bounds.height) / 8,
          )
        }
        await clickSquare('e2')
        await clickSquare('e4')
        await moves.getByText('e4').waitFor()
        record.positionAfterEngineOff = (await moves.innerText()).trim()
        record.boardPiecesAfterEngineOff = await page
          .locator('.analysis-board cg-board piece')
          .evaluateAll((pieces) =>
            pieces.map((piece) => `${piece.className} ${piece.getAttribute('style') ?? ''}`).sort(),
          )
        record.positionChangedWhileOff =
          /\be4\b/.test(record.positionAfterEngineOff) &&
          JSON.stringify(record.boardPiecesBeforeEngineReenable) !==
            JSON.stringify(record.boardPiecesAfterEngineOff)

        const reenableStarted = performance.now()
        await engineToggle.click()
        await page.waitForFunction(
          () => /Depth \d+/.test(document.querySelector('.ceval-status')?.textContent ?? ''),
          null,
          { timeout: 10_000 },
        )
        record.timingsMs.engineReenabledFirstUpdate = Math.round(
          performance.now() - reenableStarted,
        )
        record.observations.push(await captureMetrics(app, 'analysis-engine-reenabled-active'))
        await page.getByRole('button', { name: 'Engine settings', exact: true }).click()
        record.analysisEngineSettingsAfterReenable = {
          infinite: await page
            .getByRole('switch', { name: 'Infinite analysis', exact: true })
            .getAttribute('aria-checked'),
          engineLines: (
            await page.getByRole('combobox', { name: 'Engine lines' }).innerText()
          ).trim(),
        }
        await page.getByRole('button', { name: 'Engine settings', exact: true }).click()
        record.engineSettingsPreserved =
          JSON.stringify(record.analysisEngineSettings) ===
          JSON.stringify(record.analysisEngineSettingsAfterReenable)
        if (!record.engineSettingsPreserved || !record.positionChangedWhileOff) {
          throw new Error('Engine re-enable settings or changed-position check failed')
        }
        record.engineLifecycleWorkloadSucceeded = true
        record.engineWorkload =
          'started, killed on engine off, then restarted on a different position'
      } catch (error) {
        record.engineLifecycleWorkloadSucceeded = false
        record.engineWorkload = 'engine did not reach an active depth update'
        record.engineError = String(error?.message ?? error)
        try {
          if (await engineToggle.isChecked()) {
            await engineToggle.click()
            await page.waitForFunction(
              () => document.querySelector('.ceval-status')?.textContent?.trim() === 'Engine off',
              null,
              { timeout: 10_000 },
            )
            record.engineCancelledAfterFailure = true
            record.observations.push(
              await captureMetrics(app, 'analysis-engine-cancelled-after-failure'),
            )
          }
        } catch (cancelError) {
          record.engineCancelledAfterFailure = false
          record.engineCancellationError = String(cancelError?.message ?? cancelError)
        }
      }
    } else {
      record.engineLifecycleWorkloadSucceeded = false
      record.engineWorkload = (await engineStatus.textContent())?.trim() || 'engine toggle disabled'
    }

    const historyStarted = performance.now()
    await page.locator('aside').getByText('Game history', { exact: true }).click()
    await page
      .getByRole('heading', { name: 'Game history' })
      .waitFor({ state: 'visible', timeout: 15_000 })
    record.surfaceLabels.historyHeading = (
      await page.getByRole('heading', { name: 'Game history' }).innerText()
    ).trim()
    record.timingsMs.historyNavigation = Math.round(performance.now() - historyStarted)
    record.observations.push(await captureMetrics(app, 'history-settled-empty'))

    await page.locator('aside').getByText('Home', { exact: true }).click()
    await visibleStart.waitFor({ state: 'visible', timeout: 10_000 })
    record.observations.push(await captureMetrics(app, 'home-after-navigation'))
    record.finishedAt = new Date().toISOString()
    record.success = record.engineLifecycleWorkloadSucceeded === true
  } catch (error) {
    record.finishedAt = new Date().toISOString()
    record.success = false
    record.errors.push(String(error?.stack ?? error))
  } finally {
    if (app) {
      try {
        await app.close()
      } catch (error) {
        record.errors.push(`close: ${String(error?.stack ?? error)}`)
      }
    }
    try {
      await rm(profile, { recursive: true, force: true })
      record.profileRemoved = true
    } catch (error) {
      record.profileRemoved = false
      record.errors.push(`profile cleanup: ${String(error?.stack ?? error)}`)
    }
    process.stdout.write(`${JSON.stringify(record)}\n`)
    results.launches.push(record)
    if (!probeOnly || writeProbeOutput) {
      await mkdir(path.dirname(outputPath), { recursive: true })
      results.completedAt = new Date().toISOString()
      if (!probeOnly || writeProbeOutput) {
        await writeFile(outputPath, `${JSON.stringify(results, null, 2)}\n`)
      }
    }
  }
  return record
}

const expectedLabels = new Map()
for (
  let ordinal = 0;
  ordinal < (probeOnly ? variants.length : runCount * variants.length);
  ordinal += 1
) {
  const pairVariants =
    Math.floor(ordinal / variants.length) % 2 === 0 ? variants : [...variants].reverse()
  const variant = pairVariants[ordinal % variants.length]
  const runIndex = Math.floor(ordinal / variants.length) + 1
  const record = await runOne(variant, runIndex, ordinal)
  if (runIndex === 1 && record.surfaceLabels) {
    expectedLabels.set(variant.id, record.surfaceLabels)
    const bothCaptured = variants.every((item) => expectedLabels.has(item.id))
    if (bothCaptured) {
      const [rustLabels, baselineLabels] = variants.map((item) => expectedLabels.get(item.id))
      results.surfaceLabelParity = {
        rust: rustLabels,
        typescript: baselineLabels,
        matched: JSON.stringify(rustLabels) === JSON.stringify(baselineLabels),
      }
      results.completedAt = new Date().toISOString()
      await writeFile(outputPath, `${JSON.stringify(results, null, 2)}\n`)
      if (!results.surfaceLabelParity.matched) {
        throw new Error(
          `Visible surface labels differ between revisions: ${JSON.stringify(results.surfaceLabelParity)}`,
        )
      }
    }
  }
  if (runIndex === 1 && ordinal === variants.length - 1 && !results.surfaceLabelParity?.matched) {
    throw new Error(
      'Could not verify visible label parity on the first Rust and TypeScript launches; stopping before repeated measurements.',
    )
  }
}

if (results.launches.some((launch) => !launch.success)) process.exitCode = 1
