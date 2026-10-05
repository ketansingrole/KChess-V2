import { beforeEach, it, expect, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { deferred } from './fixtures'

import { OnlineSession } from '../../src/main/lichess'
import { LichessError } from '../../src/shared/lichessError'

const mocks = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  use: vi.fn(),
  getToken: vi.fn(async (): Promise<string | null> => 'Alice-token'),
  accounts: [{ username: 'Alice', connected: true }],
  readLines: vi.fn(async () => {}),
}))
vi.mock('electron', () => ({ shell: {} }))
vi.mock('openapi-fetch', () => ({ default: () => mocks }))
vi.mock('../../src/main/store', () => ({
  loadData: async () => ({ accounts: mocks.accounts }),
  getToken: mocks.getToken,
}))
vi.mock('../../src/main/usage', () => ({
  meteredFetch: vi.fn(),
  attributeTo: vi.fn(),
  withUsage: (_account: string, _kind: string, fn: () => unknown) => fn(),
}))
vi.mock('../../src/main/ndjson', () => ({ readLines: mocks.readLines }))

beforeEach(() => {
  mocks.accounts = [{ username: 'Alice', connected: true }]
  mocks.readLines.mockResolvedValue(undefined)
  mocks.getToken.mockResolvedValue('Alice-token')
  mocks.GET.mockImplementation(
    (_path: string, { signal }: { signal: AbortSignal }) =>
      new Promise((_resolve, reject) =>
        signal.addEventListener('abort', () => reject(new Error('aborted')), { once: true }),
      ),
  )
})

it('keeps assistance blocked after a failed game stream and cancellation until the server confirms no game', async () => {
  mocks.GET.mockImplementation((path: string) =>
    path === '/api/account/playing'
      ? Promise.resolve({ data: { nowPlaying: [{ gameId: 'AbCd1234' }] } })
      : Promise.reject(new LichessError(401, path)),
  )
  const state = vi.fn()
  const session = new OnlineSession(vi.fn(), vi.fn(), state)
  expect(await session.resume()).toEqual({ id: 'AbCd1234', account: 'Alice' })
  await flushPromises()
  expect(state).toHaveBeenCalledWith(
    expect.objectContaining({ phase: 'auth-required', gameId: 'AbCd1234' }),
  )
  expect(session.playing).toBe(true)
  session.cancel()
  expect(session.playing).toBe(true)
  mocks.GET.mockResolvedValue({ data: { nowPlaying: [] } })
  expect(await session.resume()).toBeNull()
  expect(session.playing).toBe(false)
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

it('retains live-game protection when the owning credentials or account disappear', async () => {
  mocks.GET.mockImplementation((path: string) =>
    path === '/api/account/playing'
      ? Promise.resolve({ data: { nowPlaying: [{ gameId: 'AbCd1234', speed: 'rapid' }] } })
      : Promise.reject(new LichessError(401, path)),
  )
  const state = vi.fn(),
    session = new OnlineSession(vi.fn(), vi.fn(), state)
  await session.resume()
  await flushPromises()
  session.cancel()
  mocks.getToken.mockResolvedValue(null)
  mocks.accounts = []
  const count = mocks.GET.mock.calls.length
  await expect(session.resume()).rejects.toThrow('login is unavailable')
  expect(mocks.GET).toHaveBeenCalledTimes(count)
  expect(session.playing).toBe(true)
  expect(state).toHaveBeenLastCalledWith(
    expect.objectContaining({ phase: 'auth-required', gameId: 'AbCd1234', account: 'Alice' }),
  )
  session.close()
})
it('ignores a cancelled resume before missing credentials arrive', async () => {
  const token = deferred<string | null>()
  mocks.getToken.mockReturnValue(token.promise)
  const state = vi.fn(),
    session = new OnlineSession(vi.fn(), vi.fn(), state)
  const pending = session.resume()
  await flushPromises()
  session.cancel()
  const count = state.mock.calls.length
  token.resolve(null)
  await expect(pending).rejects.toMatchObject({ name: 'AbortError' })
  expect(state).toHaveBeenCalledTimes(count)
  expect(mocks.GET).not.toHaveBeenCalled()
  session.close()
})

it('blocks startup assistance through pending and failed verification, then clears after an authoritative retry', async () => {
  const pending = deferred<{ data: { nowPlaying: never[] } }>()
  mocks.GET.mockReturnValue(pending.promise)
  const state = vi.fn(),
    session = new OnlineSession(vi.fn(), vi.fn(), state)
  expect(session.assistanceBlocked).toBe(true)
  expect(session.playing).toBe(false)
  const result = session.resume()
  const rejected = expect(result).rejects.toThrow()
  expect(state).toHaveBeenLastCalledWith(expect.objectContaining({ phase: 'checking' }))
  await flushPromises()
  expect(session.assistanceBlocked).toBe(true)
  pending.reject(new LichessError(503, '/api/account/playing'))
  await rejected
  expect(session.assistanceBlocked).toBe(true)
  expect(state).toHaveBeenLastCalledWith(expect.objectContaining({ phase: 'disconnected' }))
  mocks.GET.mockResolvedValue({ data: { nowPlaying: [] } })
  expect(await session.resume()).toBeNull()
  expect(session.assistanceBlocked).toBe(false)
  session.close()
})
it('clears startup protection immediately with no connected accounts', async () => {
  mocks.accounts = []
  const session = new OnlineSession(vi.fn(), vi.fn())
  expect(await session.resume()).toBeNull()
  expect(session.assistanceBlocked).toBe(false)
  expect(mocks.GET).not.toHaveBeenCalled()
  session.close()
})
it('keeps startup protection when any account is unverified or the response is malformed', async () => {
  mocks.accounts = [
    { username: 'Alice', connected: true },
    { username: 'Bob', connected: true },
  ]
  mocks.GET.mockResolvedValueOnce({ data: { nowPlaying: [] } }).mockRejectedValueOnce(
    new Error('offline'),
  )
  const session = new OnlineSession(vi.fn(), vi.fn())
  await expect(session.resume()).rejects.toThrow('offline')
  expect(session.assistanceBlocked).toBe(true)
  mocks.GET.mockResolvedValue({ data: {} })
  await expect(session.resume()).rejects.toThrow('invalid ongoing-game response')
  expect(session.assistanceBlocked).toBe(true)
  session.close()
})

it('forgets account and game recovery on explicit logout and rejects a late stream', async () => {
  const state = vi.fn()
  mocks.GET.mockImplementation((path: string) =>
    path === '/api/account/playing'
      ? Promise.resolve({ data: { nowPlaying: [{ gameId: 'AbCd1234' }] } })
      : Promise.reject(new LichessError(401, path)),
  )
  const session = new OnlineSession(vi.fn(), vi.fn(), state)
  await session.resume()
  await flushPromises()
  session.logout()
  expect(session.playing).toBe(false)
  expect(session.assistanceBlocked).toBe(false)
  expect(state).toHaveBeenLastCalledWith(
    expect.objectContaining({ account: '', gameId: '', phase: 'idle' }),
  )
  mocks.accounts = []
  await session.resume()
  expect(session.assistanceBlocked).toBe(false)
  session.close()
})

it('preserves another account’s playing session when logging out an unrelated account', async () => {
  const state = vi.fn()
  mocks.GET.mockImplementation((path: string) =>
    path === '/api/account/playing'
      ? Promise.resolve({ data: { nowPlaying: [{ gameId: 'AbCd1234' }] } })
      : Promise.reject(new LichessError(401, path)),
  )
  const session = new OnlineSession(vi.fn(), vi.fn(), state)
  await session.resume()
  await flushPromises()
  const count = state.mock.calls.length
  session.logout(['Bob'])
  expect(session.playing).toBe(true)
  expect(session.assistanceBlocked).toBe(true)
  expect(state).toHaveBeenCalledTimes(count)
  session.logout(['Alice'])
  expect(session.playing).toBe(false)
  expect(state).toHaveBeenLastCalledWith(
    expect.objectContaining({ account: '', gameId: '', phase: 'idle' }),
  )
})

it('retains protection through generated credential and network failures until authoritative recovery', async () => {
  const fc = await import('fast-check')
  await fc.assert(
    fc.asyncProperty(
      fc.array(fc.constantFrom('cancel', 'credentials', 'network'), {
        minLength: 1,
        maxLength: 10,
      }),
      async (actions) => {
        mocks.accounts = [{ username: 'Alice', connected: true }]
        mocks.getToken.mockResolvedValue('Alice-token')
        mocks.GET.mockImplementation((path: string) =>
          path === '/api/account/playing'
            ? Promise.resolve({ data: { nowPlaying: [{ gameId: 'AbCd1234' }] } })
            : Promise.reject(new Error('offline')),
        )
        const session = new OnlineSession(vi.fn(), vi.fn())
        try {
          await session.resume()
          await flushPromises()
          for (const action of actions) {
            session.cancel()
            if (action !== 'cancel') {
              mocks.getToken.mockResolvedValue(action === 'credentials' ? null : 'Alice-token')
              mocks.GET.mockRejectedValue(new Error('offline'))
              await expect(session.resume()).rejects.toThrow()
            }
            expect(session.playing).toBe(true)
            expect(session.assistanceBlocked).toBe(true)
          }
          mocks.getToken.mockResolvedValue('Alice-token')
          mocks.GET.mockResolvedValue({ data: { nowPlaying: [] } })
          await session.resume()
          expect(session.playing).toBe(false)
          expect(session.assistanceBlocked).toBe(false)
        } finally {
          session.close()
        }
      },
    ),
    { numRuns: 25 },
  )
})

it.each(['cancel', 'close', 'logout'] as const)(
  'rejects an attachment whose credentials arrive after %s',
  async (action) => {
    const token = deferred<string>()
    mocks.getToken.mockReturnValue(token.promise)
    const session = new OnlineSession(vi.fn(), vi.fn())
    const pending = session.attach('Alice', 'AbCd1234')
    const rejected = expect(pending).rejects.toMatchObject({ name: 'AbortError' })
    if (action === 'logout') session.logout(['Alice'])
    else session[action]()
    token.resolve('Alice-token')
    await rejected
    expect(mocks.GET).not.toHaveBeenCalled()
    expect(session.playing).toBe(false)
    session.close()
  },
)

it('keeps the newer attachment when an older account login arrives late', async () => {
  const alice = deferred<string>()
  mocks.getToken.mockImplementation((account?: string) =>
    account === 'Alice' ? alice.promise : Promise.resolve('Bob-token'),
  )
  const session = new OnlineSession(vi.fn(), vi.fn())
  const older = session.attach('Alice', 'AbCd1234')
  const rejected = expect(older).rejects.toMatchObject({ name: 'AbortError' })
  await session.attach('Bob', 'Other123')
  const game = mocks.GET.mock.calls.find(([path]) => path === '/api/board/game/stream/{gameId}')
  expect(game?.[1].headers).toEqual({ Authorization: 'Bearer Bob-token' })
  alice.resolve('Alice-token')
  await rejected
  expect(
    mocks.GET.mock.calls.filter(([path]) => path === '/api/board/game/stream/{gameId}'),
  ).toHaveLength(1)
  expect(game?.[1].signal.aborted).toBe(false)
  session.close()
})

it('supersedes a pending attachment with authoritative recovery', async () => {
  const token = deferred<string>()
  mocks.getToken.mockReturnValueOnce(token.promise).mockResolvedValue('Alice-token')
  mocks.GET.mockResolvedValue({ data: { nowPlaying: [] } })
  const session = new OnlineSession(vi.fn(), vi.fn())
  const pending = session.attach('Alice', 'AbCd1234')
  const rejected = expect(pending).rejects.toMatchObject({ name: 'AbortError' })
  await session.resume()
  token.resolve('Alice-token')
  await rejected
  expect(mocks.GET).toHaveBeenCalledTimes(1)
  expect(session.playing).toBe(false)
  session.close()
})

it('treats a late credential failure for a cancelled attachment as cancellation', async () => {
  const token = deferred<string>()
  mocks.getToken.mockReturnValue(token.promise)
  const session = new OnlineSession(vi.fn(), vi.fn())
  const pending = session.attach('Alice', 'AbCd1234')
  const rejected = expect(pending).rejects.toMatchObject({ name: 'AbortError' })
  session.cancel()
  token.reject(new Error('Old login failure'))
  await rejected
  expect(mocks.GET).not.toHaveBeenCalled()
  session.close()
})
