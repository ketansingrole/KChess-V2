import { beforeEach, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { OnlineSession } from '../../src/services/lichess'
import type { ChallengeInfo } from '../../src/contracts/types'

const mocks = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  use: vi.fn(),
  getToken: vi.fn(async () => 'Alice-token'),
  readLines: vi.fn(async (_stream: unknown, _onLine: (line: string) => void): Promise<void> => {}),
}))
vi.mock('openapi-fetch', () => ({ default: () => mocks }))
vi.mock('../../src/services/store', () => ({
  loadData: async () => ({ accounts: [{ username: 'Alice', connected: true }] }),
  getToken: mocks.getToken,
}))
vi.mock('../../src/services/usage', () => ({
  meteredFetch: vi.fn(),
  attributeTo: vi.fn(),
  withUsage: (_account: string, _kind: string, fn: () => unknown) => fn(),
}))
vi.mock('../../src/services/ndjson', () => ({ readLines: mocks.readLines }))

/** Streams stay open until aborted, like Lichess's. */
const openStream = (_path: string, options?: { signal?: AbortSignal }) =>
  new Promise((_resolve, reject) =>
    options?.signal?.addEventListener('abort', () => reject(new Error('aborted')), { once: true }),
  )

beforeEach(() => {
  mocks.GET.mockReset()
  mocks.POST.mockReset()
  mocks.readLines.mockReset().mockResolvedValue(undefined)
  mocks.getToken.mockResolvedValue('Alice-token')
  mocks.POST.mockResolvedValue({ data: { ok: true } })
})

/** A session attached to game AbCd1234 of Alice. */
async function playing(): Promise<OnlineSession> {
  mocks.GET.mockImplementation((path: string, options?: { signal?: AbortSignal }) =>
    path === '/api/account/playing'
      ? Promise.resolve({
          data: {
            nowPlaying: [
              { gameId: 'Corr1234', speed: 'correspondence' },
              { gameId: 'AbCd1234', speed: 'rapid' },
            ],
          },
        })
      : openStream(path, options),
  )
  const session = new OnlineSession(vi.fn(), vi.fn())
  expect(await session.resume()).toEqual({ id: 'AbCd1234', account: 'Alice' })
  return session
}

it('reattaches to the live game, not a correspondence one', async () => {
  const session = await playing()
  expect(session.playing).toBe(true)
  session.cancel()
})

it.each([
  ['offerDraw', '/api/board/game/{gameId}/draw/{accept}', 'yes'],
  ['acceptDraw', '/api/board/game/{gameId}/draw/{accept}', 'yes'],
  ['declineDraw', '/api/board/game/{gameId}/draw/{accept}', false],
  ['claimVictory', '/api/board/game/{gameId}/claim-victory', undefined],
  ['claimDraw', '/api/board/game/{gameId}/claim-draw', undefined],
  ['berserk', '/api/board/game/{gameId}/berserk', undefined],
] as const)('%s calls %s', async (action, path, accept) => {
  const session = await playing()
  await session.action('AbCd1234', action)
  const call = mocks.POST.mock.calls.find(([called]) => called === path)
  expect(call?.[1].params.path).toEqual(
    accept === undefined ? { gameId: 'AbCd1234' } : { gameId: 'AbCd1234', accept },
  )
  expect(call?.[1].headers).toEqual({ Authorization: 'Bearer Alice-token' })
  session.cancel()
})

it('refuses actions and chat for a game not on the board', async () => {
  const session = await playing()
  await expect(session.action('Other123', 'offerDraw')).rejects.toThrow('Reconnect')
  await expect(session.sendChat('Other123', 'player', 'hi')).rejects.toThrow('Reconnect')
  session.cancel()
})

it('sends and reads the player chat of the live game', async () => {
  const session = await playing()
  await session.sendChat('AbCd1234', 'player', 'Good luck')
  const sent = mocks.POST.mock.calls.find(([path]) => path === '/api/board/game/{gameId}/chat')
  expect(sent?.[1].body).toEqual({ room: 'player', text: 'Good luck' })
  mocks.GET.mockImplementation((path: string) =>
    path === '/api/board/game/{gameId}/chat'
      ? Promise.resolve({ data: [{ user: 'Bob', text: 'You too' }, { bogus: 1 }] })
      : openStream(path),
  )
  expect(await session.chat('AbCd1234')).toEqual([{ user: 'Bob', text: 'You too', room: 'player' }])
  session.cancel()
})

it('keeps the idle event stream open for challenges and closes it on request', async () => {
  mocks.GET.mockImplementation(openStream)
  const session = new OnlineSession(vi.fn(), vi.fn())
  session.stayConnected('Alice')
  await flushPromises()
  const call = mocks.GET.mock.calls.find(([path]) => path === '/api/stream/event')
  expect(call?.[1].headers).toEqual({ Authorization: 'Bearer Alice-token' })
  const signal = call?.[1].signal as AbortSignal
  expect(signal.aborted).toBe(false)
  session.stayConnected('')
  expect(signal.aborted).toBe(true)
})

it('hands challenge events on the idle stream to the inbox and opens accepted games', async () => {
  const challenge = vi.fn()
  let deliver: ((line: string) => void) | undefined
  mocks.GET.mockImplementation((path: string, options?: { signal?: AbortSignal }) =>
    path === '/api/stream/event' ? Promise.resolve({ data: {} }) : openStream(path, options),
  )
  mocks.readLines.mockImplementation(
    (_stream: unknown, onLine: (line: string) => void) =>
      new Promise<void>(() => {
        deliver = onLine
      }),
  )
  const session = new OnlineSession(vi.fn(), vi.fn(), vi.fn(), { challenge })
  session.stayConnected('Alice')
  await flushPromises()
  deliver?.(JSON.stringify({ type: 'challenge', challenge: { id: 'Chal1234' } }))
  expect(challenge).toHaveBeenCalledWith('Alice', expect.objectContaining({ type: 'challenge' }))
  deliver?.(JSON.stringify({ type: 'gameStart', game: { gameId: 'Chal1234', speed: 'rapid' } }))
  await flushPromises()
  expect(mocks.GET).toHaveBeenCalledWith(
    '/api/board/game/stream/{gameId}',
    expect.objectContaining({ params: { path: { gameId: 'Chal1234' } } }),
  )
  expect(session.playing).toBe(true)
  session.close()
})

const incoming = (overrides: Partial<ChallengeInfo> = {}): ChallengeInfo => ({
  id: 'Chal1234',
  account: 'Alice',
  direction: 'in',
  opponent: { name: 'Bob' },
  variant: 'standard',
  variantName: 'Standard',
  rated: false,
  speed: 'rapid',
  timeControl: { type: 'clock', limit: 600, increment: 0 },
  color: 'random',
  playable: true,
  receivedAt: 0,
  ...overrides,
})

it('accepts a challenge as its account and opens the game at once', async () => {
  mocks.GET.mockImplementation(openStream)
  const session = new OnlineSession(vi.fn(), vi.fn())
  await session.acceptChallenge(incoming())
  expect(mocks.POST).toHaveBeenCalledWith(
    '/api/challenge/{challengeId}/accept',
    expect.objectContaining({ params: { path: { challengeId: 'Chal1234' } } }),
  )
  expect(session.playing).toBe(true)
  session.cancel()
})

it('accepting a correspondence challenge leaves the board alone', async () => {
  const ongoingChanged = vi.fn()
  mocks.GET.mockImplementation(openStream)
  const session = new OnlineSession(vi.fn(), vi.fn(), vi.fn(), { ongoingChanged })
  await session.acceptChallenge(incoming({ timeControl: { type: 'correspondence', days: 3 } }))
  expect(session.playing).toBe(false)
  expect(ongoingChanged).toHaveBeenCalled()
})

it('declines with a reason and refuses a second live game', async () => {
  const session = await playing()
  await expect(session.acceptChallenge(incoming())).rejects.toThrow('Finish the game')
  await session.declineChallenge(incoming(), 'later')
  expect(mocks.POST).toHaveBeenCalledWith(
    '/api/challenge/{challengeId}/decline',
    expect.objectContaining({ body: { reason: 'later' } }),
  )
  session.cancel()
})

it('creates correspondence challenges without waiting on a stream', async () => {
  mocks.POST.mockResolvedValue({ data: { id: 'Corr5678', url: 'https://lichess.org/Corr5678' } })
  const session = new OnlineSession(vi.fn(), vi.fn())
  const result = await session.start({
    minutes: 0,
    increment: 0,
    color: 'random',
    rated: false,
    target: 'Bob',
    days: 3,
    variant: 'chess960',
  })
  expect(result).toMatchObject({ id: 'Corr5678', correspondence: true })
  const [, options] = mocks.POST.mock.calls[0]!
  expect(options.body).toMatchObject({ days: 3, variant: 'chess960' })
  expect(mocks.GET).not.toHaveBeenCalledWith('/api/stream/event', expect.anything())
})

it('refuses a rated or public game from a set-up position', async () => {
  const session = new OnlineSession(vi.fn(), vi.fn())
  const fen = '4k3/8/8/8/8/8/4P3/4K3 w - - 0 1'
  await expect(
    session.start({ minutes: 10, increment: 0, color: 'white', rated: true, target: 'Bob', fen }),
  ).rejects.toThrow('casual challenge')
  await expect(
    session.start({ minutes: 10, increment: 0, color: 'white', rated: false, fen }),
  ).rejects.toThrow('casual challenge')
})
