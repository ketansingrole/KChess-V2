import { readFile, writeFile, mkdir, appendFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { regressionMetrics } from './regression-metrics.mjs'

export function desktopMetrics(report) {
  const { expected = 0, unexpected = 0, flaky = 0, skipped = 0 } = report.stats ?? {}
  const attempted = expected + unexpected + flaky
  return {
    passed: expected,
    failed: unexpected,
    flaky,
    skipped,
    flakyRate: attempted ? flaky / attempted : null,
  }
}

async function read(path) {
  try {
    return JSON.parse(await readFile(path, 'utf8'))
  } catch (cause) {
    if (cause.code === 'ENOENT') return null
    throw cause
  }
}

export async function reportGuardrails() {
  const full = await read('test-results/guardrails/check-full.json')
  const fast = await read('test-results/guardrails/check-fast.json')
  const library = await read('test-results/guardrails/library.json')
  const desktop = await read('test-results/desktop.json')
  const packaged = await read('test-results/desktop-packaged.json')
  let regressions = null
  if (process.env.GITHUB_REPOSITORY && process.env.GH_TOKEN) {
    try {
      regressions = regressionMetrics(process.env.GITHUB_REPOSITORY)
    } catch {
      regressions = { count: null, reason: 'GitHub regression metrics could not be read.' }
    }
  }
  const report = {
    regressions,
    checks: { full, fast },
    library,
    desktop: desktop ? desktopMetrics(desktop) : null,
    packaged: packaged ? desktopMetrics(packaged) : null,
  }
  await mkdir('test-results/guardrails', { recursive: true })
  await writeFile('test-results/guardrails/summary.json', JSON.stringify(report, null, 2) + '\n')
  const lines = [
    '## Guardrail measurements',
    '',
    '| Check | Duration | Result |',
    '| --- | ---: | --- |',
  ]
  for (const check of [full, fast].filter(Boolean)) {
    for (const step of check.steps)
      lines.push(
        '| ' +
          check.mode +
          ': ' +
          step.name +
          ' | ' +
          (step.milliseconds / 1000).toFixed(2) +
          ' s | ' +
          (step.exitCode ? 'failed' : 'passed') +
          ' |',
      )
  }
  for (const [name, metrics] of [
    ['Desktop', report.desktop],
    ['Packaged', report.packaged],
  ]) {
    if (metrics)
      lines.push(
        '',
        name +
          ': ' +
          metrics.passed +
          ' passed, ' +
          metrics.failed +
          ' failed, ' +
          metrics.flaky +
          ' flaky (' +
          (metrics.flakyRate === null
            ? 'unavailable'
            : (metrics.flakyRate * 100).toFixed(1) + '%') +
          ').',
      )
  }
  if (library) {
    lines.push(
      '',
      'Library timings are recorded as trends; row counts and fixture transfer size are blocking budgets.',
    )
    for (const sample of library.samples)
      lines.push(
        '- ' +
          sample.count +
          ' games: ' +
          sample.pageQueryMs +
          ' ms page query, ' +
          sample.pageSerializedBytes +
          ' bytes for ' +
          sample.pageRows +
          ' rows.',
      )
  }
  if (regressions)
    lines.push(
      '',
      regressions.count === null
        ? 'Released regressions: unavailable. ' + regressions.reason
        : 'Confirmed released regressions reported since ' +
            regressions.since +
            ': ' +
            regressions.count +
            (regressions.truncated ? '+ (query limit reached)' : '') +
            '.',
    )
  const markdown = lines.join('\n') + '\n'
  console.info(markdown)
  if (process.env.GITHUB_STEP_SUMMARY) await appendFile(process.env.GITHUB_STEP_SUMMARY, markdown)
  return report
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await reportGuardrails()
