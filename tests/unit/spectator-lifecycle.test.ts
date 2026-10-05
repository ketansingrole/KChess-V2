import { expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { Spectator } from '../../src/main/spectate'
import { deferred } from './fixtures'
const mocks = vi.hoisted(() => ({ GET: vi.fn(), POST: vi.fn(), use: vi.fn(), readLines: vi.fn() }))
vi.mock('electron', () => ({ shell: {} }))
vi.mock('openapi-fetch', () => ({ default: () => mocks }))
vi.mock('../../src/main/store', () => ({ getToken: vi.fn() }))
vi.mock('../../src/main/usage', () => ({
  meteredFetch: vi.fn(),
  withUsage: (_a: string, _k: string, fn: () => unknown) => fn(),
}))
vi.mock('../../src/main/ndjson', () => ({ readLines: mocks.readLines }))
it('reports an initial HTTP failure after the start call has returned', async () => {
  mocks.GET.mockRejectedValue(new Error('Offline'))
  const state = vi.fn(),
    spectator = new Spectator(vi.fn(), vi.fn(), state)
  const session = spectator.watch({ channel: 'rapid' })
  await flushPromises()
  expect(state).toHaveBeenLastCalledWith({ session, phase: 'error', message: 'Offline' })
  spectator.stop()
})
it('reports a stalled feed but suppresses errors from a cancelled session', async () => {
  mocks.GET.mockResolvedValue({ data: new ReadableStream(), response: new Response() })
  const stalled = deferred<undefined>()
  mocks.readLines.mockReturnValue(stalled.promise)
  const state = vi.fn(),
    spectator = new Spectator(vi.fn(), vi.fn(), state)
  const session = spectator.watch({ gameId: 'Game0001' })
  await flushPromises()
  stalled.reject(new Error('Lichess stream stalled. Reconnecting…'))
  await flushPromises()
  expect(state).toHaveBeenLastCalledWith(expect.objectContaining({ session, phase: 'error' }))
  const cancelled = deferred<undefined>()
  mocks.readLines.mockReturnValue(cancelled.promise)
  spectator.watch({ gameId: 'Game0002' })
  await flushPromises()
  spectator.stop()
  const calls = state.mock.calls.length
  cancelled.reject(new Error('Aborted'))
  await flushPromises()
  expect(state).toHaveBeenCalledTimes(calls)
})
