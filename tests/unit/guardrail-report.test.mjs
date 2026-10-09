import { expect, it } from 'vitest'
import { desktopMetrics } from '../../tooling/guardrail-report.mjs'

it('counts recovered retries as flaky rather than clean passes', () => {
  expect(desktopMetrics({ stats: { expected: 8, unexpected: 1, flaky: 1, skipped: 5 } })).toEqual({
    passed: 8,
    failed: 1,
    flaky: 1,
    skipped: 5,
    flakyRate: 0.1,
  })
})
it('reports an unavailable flake rate when no tests attempted', () => {
  expect(desktopMetrics({ stats: { skipped: 4 } }).flakyRate).toBeNull()
})
