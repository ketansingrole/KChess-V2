#!/usr/bin/env node
// Attribute packaged desktop RSS deltas to Node/V8, Chromium renderer, and WASM memory.
// Always start this script through tooling/with-branded-electron.mjs.
import { _electron } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
const roots = process.argv.slice(2).filter((arg) => !arg.startsWith('--'))
const runArg = process.argv.find((arg) => arg.startsWith('--runs='))
const runCount = Number(runArg?.split('=')[1] ?? 3)
const gcDiagnostic = process.argv.includes('--gc-diagnostic')
const instrumentWasm = process.argv.includes('--instrument-wasm')
const outputName = gcDiagnostic
  ? 'renderer-memory-gc.json'
  : instrumentWasm
    ? 'renderer-memory-wasm.json'
    : 'renderer-memory.json'
const outputPath = path.join(root, 'docs/benchmarks/raw', outputName)
if (!process.env.ELECTRON_EXEC_PATH)
  throw new Error(
    'Use node tooling/with-branded-electron.mjs node tooling/benchmarks/renderer-memory.mjs ...',
  )
if (roots.length !== 2 || !Number.isInteger(runCount) || runCount < 3)
  throw new Error(
    'Usage: renderer-memory.mjs <rust-root> <typescript-baseline-root> [--runs=3+] [--gc-diagnostic] [--instrument-wasm]',
  )
if (gcDiagnostic && instrumentWasm)
  throw new Error(
    'Run --gc-diagnostic and --instrument-wasm separately so each diagnostic has its own control.',
  )

const variants = roots.map((repoRoot, index) => {
  const resolved = path.resolve(repoRoot)
  const executablePath = path.join(resolved, 'dist/mac-arm64/KChess.app/Contents/MacOS/KChess')
  return {
    id: index === 0 ? 'rust' : 'typescript',
    root: resolved,
    revision:
      index === 0
        ? execFileSync('git', ['-C', resolved, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
        : 'f479f7c',
    executablePath,
    executableSha256: execFileSync('shasum', ['-a', '256', executablePath], { encoding: 'utf8' })
      .trim()
      .split(/\s+/)[0],
  }
})

const wasmCaptureInitScript = `(() => {
  const memories = new Set();
  const capture = (result) => {
    const instance = result && result.instance ? result.instance : result;
    const memory = instance && instance.exports && instance.exports.memory;
    if (memory && memory.buffer instanceof ArrayBuffer) memories.add(memory);
    return result;
  };
  const originalInstantiate = WebAssembly.instantiate.bind(WebAssembly);
  WebAssembly.instantiate = async (...args) => capture(await originalInstantiate(...args));
  if (typeof WebAssembly.instantiateStreaming === 'function') {
    const originalStreaming = WebAssembly.instantiateStreaming.bind(WebAssembly);
    WebAssembly.instantiateStreaming = async (...args) => capture(await originalStreaming(...args));
  }
  Object.defineProperty(window, '__kchessWasmMemoryBytes', {
    configurable: false,
    get: () => [...memories].map((memory) => memory.buffer.byteLength),
  });
})()`

const raw = {
  schemaVersion: 1,
  status: 'prepared',
  startedAt: null,
  completedAt: null,
  host: {
    platform: process.platform,
    release: os.release(),
    architecture: os.arch(),
    cpuModel: os.cpus()[0]?.model ?? null,
    logicalCpuCount: os.cpus().length,
    nodeVersion: process.version,
    electronExecutable: process.env.ELECTRON_EXEC_PATH,
  },
  configuration: { runCount, gcDiagnostic, instrumentWasm },
  methodology: {
    variants:
      'Packaged Rust @ current checkout vs extracted TypeScript baseline @ f479f7c. Each launch uses a new disposable profile and no connected account.',
    launch:
      'Unsigned macOS arm64 KChess.app launched with Playwright through tooling/with-branded-electron.mjs; KCHESS_NUXT_URL is empty.',
    phases:
      'Home idle after 2.5 s; Analysis board with Engine analysis off; Analysis board with explicit Infinite analysis active after depth begins. Three samples per phase, one second apart.',
    attribution:
      'Electron getAppMetrics plus descendant ps RSS identify process groups. Main-process process.memoryUsage separates Node heap, external, and ArrayBuffers. Renderer CDP Runtime.getHeapUsage and Performance.getMetrics capture V8 heap/backing storage. Optional WASM init-script instrumentation reports exported WebAssembly.Memory buffer sizes; this diagnostic wrapper is run separately because it patches instantiate methods.',
    gc: 'Optional --gc-diagnostic adds a renderer CDP HeapProfiler.collectGarbage checkpoint after primary samples at each phase and records before/after heap/process memory. This is diagnostic and must not be mixed with ordinary measurements.',
    limits:
      'RSS includes allocator slack, executable mappings, JIT/compiled code, shared libraries and shared pages. Node external/arrayBuffers do not include every native-addon or WebAssembly allocation. WASM linear buffer size does not include compiled code. Process RSS sums may double-count shared pages. This study attributes memory categories but does not prove which source allocation causes a delta without further ablation.',
    order:
      'Paired fresh launches with alternating order: Rust then TypeScript in odd pairs, TypeScript then Rust in even pairs.',
  },
  variants,
  runs: [],
}

function processTreeSnapshot(rootPid, metrics) {
  const output = execFileSync('ps', ['-axo', 'pid=,ppid=,rss=,command='], {
    encoding: 'utf8',
    maxBuffer: 8 * 1024 * 1024,
  })
  const rows = []
  for (const line of output.split('\n')) {
    const match = line.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+(.*)$/)
    if (!match) continue
    rows.push({
      pid: Number(match[1]),
      ppid: Number(match[2]),
      rssKiB: Number(match[3]),
      command: match[4].slice(0, 500),
    })
  }
  const byParent = new Map()
  for (const row of rows) {
    const children = byParent.get(row.ppid) ?? []
    children.push(row)
    byParent.set(row.ppid, children)
  }
  const processRows = []
  const visit = (pid) => {
    for (const child of byParent.get(pid) ?? []) {
      processRows.push(child)
      visit(child.pid)
    }
  }
  const rootRow = rows.find((row) => row.pid === rootPid)
  if (rootRow) processRows.push(rootRow)
  visit(rootPid)
  const electronTypes = new Map(metrics.map((metric) => [metric.pid, metric.type]))
  const groups = { main: [], renderer: [], gpu: [], engine: [], other: [] }
  for (const row of processRows) {
    const type = electronTypes.get(row.pid)
    const name = row.command.toLowerCase()
    const group =
      type === 'Browser'
        ? 'main'
        : type === 'Tab'
          ? 'renderer'
          : type === 'GPU'
            ? 'gpu'
            : /stockfish|engine.*\.js/.test(name)
              ? 'engine'
              : 'other'
    groups[group].push(row)
  }
  return {
    source:
      'ps -axo pid,ppid,rss,command; descendants of Electron main PID; RSS may double-count shared pages.',
    totals: Object.fromEntries(
      Object.entries(groups).map(([key, values]) => [
        key,
        {
          processCount: values.length,
          rssKiB: values.reduce((sum, row) => sum + row.rssKiB, 0),
        },
      ]),
    ),
    processes: processRows,
  }
}

async function capture(app, page, cdp, label) {
  const [metrics, heap, performance, main] = await Promise.all([
    app.evaluate(({ app: electronApp }) => electronApp.getAppMetrics()),
    cdp.send('Runtime.getHeapUsage'),
    cdp.send('Performance.getMetrics'),
    app.evaluate(({ BrowserWindow }) => {
      const win = BrowserWindow.getAllWindows()[0]
      const memory = process.memoryUsage()
      return {
        pid: process.pid,
        processMemoryUsage: {
          rssBytes: memory.rss,
          heapTotalBytes: memory.heapTotal,
          heapUsedBytes: memory.heapUsed,
          externalBytes: memory.external,
          arrayBuffersBytes: memory.arrayBuffers,
        },
        window: win
          ? { webContentsId: win.webContents.id, rendererPid: win.webContents.getOSProcessId() }
          : null,
      }
    }),
  ])
  const pageExtras = await page.evaluate(() => ({
    url: location.href,
    title: document.title,
    performanceMemory: performance.memory
      ? {
          usedJSHeapSize: performance.memory.usedJSHeapSize,
          totalJSHeapSize: performance.memory.totalJSHeapSize,
          jsHeapSizeLimit: performance.memory.jsHeapSizeLimit,
        }
      : null,
    wasmLinearMemoryBytes: window.__kchessWasmMemoryBytes ?? null,
  }))
  return {
    label,
    measuredAt: new Date().toISOString(),
    main,
    electronProcesses: metrics.map((metric) => ({
      pid: metric.pid,
      type: metric.type,
      name: metric.name ?? null,
      serviceName: metric.serviceName ?? null,
      memory: metric.memory
        ? { workingSetKiB: metric.memory.workingSetSize, privateKiB: metric.memory.privateBytes }
        : null,
      cpu: metric.cpu
        ? {
            percentCPUUsage: metric.cpu.percentCPUUsage,
            cumulativeCPUUsage: metric.cpu.cumulativeCPUUsage,
          }
        : null,
    })),
    processTree: processTreeSnapshot(app.process().pid, metrics),
    renderer: {
      pid: main.window?.rendererPid ?? null,
      cdpHeapUsage: heap,
      pagePerformanceMemory: pageExtras.performanceMemory,
      page: { url: pageExtras.url, title: pageExtras.title },
      wasmLinearMemoryBytes: pageExtras.wasmLinearMemoryBytes,
      performanceMetrics: performance.metrics.filter((metric) =>
        /Heap|Nodes|Documents|LayoutCount|TaskDuration|ScriptDuration/.test(metric.name),
      ),
    },
  }
}

async function collect(app, page, cdp, run, label) {
  const samples = []
  for (let index = 0; index < 3; index += 1) {
    samples.push(await capture(app, page, cdp, `${label}-${index + 1}`))
    if (index < 2) await page.waitForTimeout(1000)
  }
  run.observations.push({ phase: label, samples })
  if (gcDiagnostic) {
    const before = await capture(app, page, cdp, `${label}-before-gc`)
    const gcStart = performance.now()
    await cdp.send('HeapProfiler.collectGarbage')
    const rendererGcElapsedMs = performance.now() - gcStart
    const after = await capture(app, page, cdp, `${label}-after-renderer-gc`)
    const mainGc = await app.evaluate(() => {
      if (typeof global.gc !== 'function') return { available: false }
      const before = process.memoryUsage()
      global.gc()
      const after = process.memoryUsage()
      return { available: true, before, after }
    })
    run.gcDiagnostics.push({ phase: label, before, after, rendererGcElapsedMs, mainGc })
  }
}

async function runOne(variant, pair, order) {
  const profile = await mkdtemp(path.join(os.tmpdir(), `kchess-renderer-memory-${variant.id}-`))
  const run = {
    variant: variant.id,
    revision: variant.revision,
    pair,
    order,
    profile: 'fresh disposable profile',
    engineConfig: { infinite: true, hashMb: 128, multiPv: 3 },
    observations: [],
    gcDiagnostics: [],
    errors: [],
  }
  let app
  try {
    app = await _electron.launch({
      executablePath: variant.executablePath,
      args: [],
      cwd: variant.root,
      timeout: 60_000,
      env: { ...process.env, KCHESS_NUXT_URL: '', KCHESS_USER_DATA_DIR: profile },
    })
    const page = await app.firstWindow()
    await page.locator('section[aria-label="Start"]').waitFor({ state: 'visible', timeout: 45_000 })
    const cdp = await page.context().newCDPSession(page)
    await cdp.send('Runtime.enable')
    await cdp.send('Performance.enable')
    if (gcDiagnostic) await cdp.send('HeapProfiler.enable')
    if (instrumentWasm) {
      await page.addInitScript(wasmCaptureInitScript)
      await page.reload({ waitUntil: 'domcontentloaded' })
      await page
        .locator('section[aria-label="Start"]')
        .waitFor({ state: 'visible', timeout: 45_000 })
    }
    await page.waitForTimeout(2500)
    await collect(app, page, cdp, run, 'home-idle')

    await page.locator('aside').getByText('Analysis board', { exact: true }).click()
    await page
      .getByRole('heading', { name: 'Analysis board' })
      .waitFor({ state: 'visible', timeout: 15_000 })
    const engineToggle = page.getByRole('switch', { name: 'Engine analysis' })
    const engineStatus = page.locator('.ceval-status')
    if (await engineToggle.isChecked()) {
      await engineToggle.click()
      await page.waitForFunction(
        () => document.querySelector('.ceval-status')?.textContent?.trim() === 'Engine off',
        null,
        { timeout: 10_000 },
      )
    }
    await collect(app, page, cdp, run, 'analysis-engine-off')

    await engineToggle.click()
    await page.waitForFunction(
      () => /Depth \d+/.test(document.querySelector('.ceval-status')?.textContent ?? ''),
      null,
      { timeout: 20_000 },
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
    run.engineStatusAtActive = (await engineStatus.textContent())?.trim()
    await collect(app, page, cdp, run, 'analysis-engine-active')

    await engineToggle.click()
    await page.waitForFunction(
      () => document.querySelector('.ceval-status')?.textContent?.trim() === 'Engine off',
      null,
      { timeout: 10_000 },
    )
    run.success = true
  } catch (error) {
    run.success = false
    run.errors.push(String(error?.stack ?? error))
  } finally {
    if (app) {
      try {
        await app.close()
      } catch (error) {
        run.errors.push(`close: ${String(error?.message ?? error)}`)
      }
    }
    await rm(profile, { recursive: true, force: true })
    run.finishedAt = new Date().toISOString()
    raw.runs.push(run)
    raw.status = 'running'
    raw.completedAt = new Date().toISOString()
    await mkdir(path.dirname(outputPath), { recursive: true })
    await writeFile(outputPath, `${JSON.stringify(raw, null, 2)}\n`)
    process.stdout.write(
      `${JSON.stringify({ variant: run.variant, pair, success: run.success, errors: run.errors })}\n`,
    )
  }
  return run
}

raw.status = 'running'
raw.startedAt = new Date().toISOString()
for (let pair = 1; pair <= runCount; pair += 1) {
  const order = pair % 2 ? variants : [...variants].reverse()
  for (const variant of order)
    await runOne(
      variant,
      pair,
      order.map((entry) => entry.id),
    )
}
raw.status = raw.runs.every((run) => run.success) ? 'complete' : 'failed'
raw.completedAt = new Date().toISOString()
await writeFile(outputPath, `${JSON.stringify(raw, null, 2)}\n`)
if (raw.status !== 'complete') process.exitCode = 1
process.stdout.write(`${outputPath}\n`)
