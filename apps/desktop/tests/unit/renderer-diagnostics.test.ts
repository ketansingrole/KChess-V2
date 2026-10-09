import { expect, it, vi } from 'vitest'
import { diagnosticRoute } from '../../contracts/rendererDiagnostics'
import { desktop } from './fixtures'

it('keeps diagnostic route context fixed and excludes query and unknown paths', () => {
  expect(diagnosticRoute('/analysis')).toBe('/analysis')
  expect(diagnosticRoute('/players?name=Alice')).toBe('/')
  expect(diagnosticRoute('/private-game-id')).toBe('/')
})

it('records one navigation sample per completion and bounds error context', async () => {
  const hooks = new Map<string, (...args: unknown[]) => void>()
  const currentRoute = { value: { path: '/analysis', fullPath: '/analysis?fen=private-position' } }
  vi.stubGlobal('defineNuxtPlugin', (setup: unknown) => setup)
  vi.stubGlobal('useRouter', () => ({ currentRoute }))
  const recordPerformance = vi.fn(async () => {})
  const reportRendererError = vi.fn(async () => {})
  desktop({ recordPerformance, reportRendererError })
  const plugin = (await import('../../app/plugins/diagnostics.client'))
    .default as unknown as (app: {
    hook: (name: string, callback: (...args: unknown[]) => void) => void
  }) => void
  plugin({
    hook: (name, callback) => {
      hooks.set(name, callback)
    },
  })
  const now = vi.spyOn(performance, 'now').mockReturnValueOnce(10).mockReturnValueOnce(35)
  hooks.get('page:loading:end')!()
  expect(recordPerformance).not.toHaveBeenCalled()
  hooks.get('page:loading:start')!()
  hooks.get('page:loading:end')!()
  hooks.get('page:loading:end')!()
  expect(recordPerformance).toHaveBeenCalledExactlyOnceWith('page.navigation:/analysis', 25)
  hooks.get('vue:error')!(new Error('x'.repeat(2000)), {}, 'y'.repeat(200))
  expect(reportRendererError).toHaveBeenCalledExactlyOnceWith({
    route: '/analysis',
    message: 'x'.repeat(1000),
    info: 'y'.repeat(160),
  })
  expect(JSON.stringify(reportRendererError.mock.calls)).not.toContain('private-position')
  now.mockRestore()
})
