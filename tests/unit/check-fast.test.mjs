import { expect, it } from 'vitest'
import { selectChecks } from '../../tooling/check-fast.mjs'
import { assertLibraryBudget } from '../../tooling/performance-budgets.mjs'

it('runs only changed formatting, lint and related tests for renderer edits', () => {
  const steps = selectChecks(['apps/desktop/app/stores/watch.ts'])
  expect(steps.map(([name]) => name)).toEqual(['format', 'lint', 'unit-related'])
  expect(steps.at(-1)[1]).toContain('apps/desktop/app/stores/watch.ts')
})
it.each([
  'core/src/contracts/types.ts',
  'core/src/services/platform.ts',
  'pnpm-lock.yaml',
  'pnpm-workspace.yaml',
  'vitest.config.ts',
  'tooling/build.mjs',
  '.github/workflows/ci.yml',
])('broadens verification when %s changes', (file) => {
  expect(selectChecks([file]).map(([name]) => name)).toEqual(
    expect.arrayContaining(['types', 'unit']),
  )
})
it('runs new regression tests directly and tolerates asset-only changes', () => {
  expect(selectChecks(['tests/unit/new.test.ts']).at(-1)[1]).toContain('tests/unit/new.test.ts')
  expect(selectChecks(['apps/desktop/public/icon.png']).map(([name]) => name)).toEqual(['format'])
  expect(selectChecks([])).toEqual([])
})
it('fails structural performance regressions independently of wall-clock speed', () => {
  expect(() => assertLibraryBudget({ pageRows: 100, pageSerializedBytes: 30_000 })).not.toThrow()
  expect(() => assertLibraryBudget({ pageRows: 1000, pageSerializedBytes: 30_000 })).toThrow(
    'bounded',
  )
  expect(() => assertLibraryBudget({ pageRows: 100, pageSerializedBytes: 100_000 })).toThrow(
    'transfer',
  )
})
