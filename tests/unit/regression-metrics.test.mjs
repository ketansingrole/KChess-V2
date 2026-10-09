import { expect, it, vi } from 'vitest'
import { regressionMetrics } from '../../tooling/regression-metrics.mjs'

it('counts confirmed released regressions across open and closed issues', () => {
  const execute = vi
    .fn()
    .mockReturnValueOnce(JSON.stringify([{ name: 'regression' }]))
    .mockReturnValueOnce(JSON.stringify([{ number: 42, createdAt: '2026-10-01T00:00:00Z' }]))
  expect(regressionMetrics('owner/repo', execute, Date.UTC(2026, 9, 5))).toEqual({
    since: '2026-09-05',
    count: 1,
    truncated: false,
  })
  expect(execute.mock.calls[1][1]).toEqual(
    expect.arrayContaining(['all', 'regression', 'created:>=2026-09-05']),
  )
})
it('does not claim zero when regression classification is absent', () => {
  const execute = vi.fn(() => '[]')
  expect(regressionMetrics('owner/repo', execute).count).toBeNull()
  expect(execute).toHaveBeenCalledTimes(1)
})
