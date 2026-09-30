import { beforeEach, it, expect, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { deferred } from './fixtures'

import { OnlineSession } from '../../src/main/lichess'

const mocks = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  use: vi.fn(),
  getToken: vi.fn(async () => 'Alice-token'),
}))
vi.mock('electron', () => ({ shell: {} }))
vi.mock('openapi-fetch', () => ({ default: () => mocks }))
vi.mock('../../src/main/store', () => ({
  loadData: async () => ({ accounts: [{ username: 'Alice', connected: true }] }),
  getToken: mocks.getToken,
}))
vi.mock('../../src/main/usage', () => ({
  meteredFetch: vi.fn(),
  attributeTo: vi.fn(),
  withUsage: (_account: string, _kind: string, fn: () => unknown) => fn(),
}))
vi.mock('../../src/main/ndjson', () => ({ readLines: async () => {} }))

beforeEach(() => {
  mocks.getToken.mockResolvedValue('Alice-token')
  mocks.GET.mockImplementation(
    (_path: string, { signal }: { signal: AbortSignal }) =>
      new Promise((_resolve, reject) =>
        signal.addEventListener('abort', () => reject(new Error('aborted')), { once: true }),
      ),
  )
})

it('aborts an outstanding challenge and cancels a late challenge creation', async () => {
  const pending = deferred<{ data: { id: string; url: string } }>()
  mocks.POST.mockImplementation((path: string) =>
    path === '/api/challenge/{username}'
      ? pending.promise
      : Promise.resolve({ data: { ok: true } }),
  )
  const session = new OnlineSession(vi.fn(), vi.fn())
  const result = session.start({
    minutes: 10,
    increment: 0,
    color: 'random',
    rated: false,
    target: 'Bob',
  })
  const rejection = expect(result).rejects.toMatchObject({ name: 'AbortError' })
  await flushPromises()
  session.cancel()
  expect(mocks.POST.mock.calls[0]?.[1].signal.aborted).toBe(true)
  pending.resolve({ data: { id: 'AbCd1234', url: 'https://lichess.org/AbCd1234' } })
  await rejection
  expect(mocks.POST.mock.calls[1]).toMatchObject([
    '/api/challenge/{challengeId}/cancel',
    {
      params: { path: { challengeId: 'AbCd1234' } },
      headers: { Authorization: 'Bearer Alice-token' },
    },
  ])
  session.cancel()
  await flushPromises()
  expect(mocks.POST).toHaveBeenCalledTimes(2)
})

it('prevents streams and challenges when cancelled before credentials arrive', async () => {
  const login = deferred<string>()
  mocks.getToken.mockReturnValue(login.promise)
  const session = new OnlineSession(vi.fn(), vi.fn())
  const result = session.start({
    minutes: 10,
    increment: 0,
    color: 'random',
    rated: false,
    target: 'Bob',
  })
  const rejection = expect(result).rejects.toMatchObject({ name: 'AbortError' })
  await flushPromises()
  session.cancel()
  login.resolve('Alice-token')
  await rejection
  expect(mocks.GET).not.toHaveBeenCalled()
  expect(mocks.POST).not.toHaveBeenCalled()
})
