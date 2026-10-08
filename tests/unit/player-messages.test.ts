import { beforeEach, expect, it, vi } from 'vitest'
import { OAUTH_SCOPES, recentGames, sendMessage } from '../../src/core/lichess'
import { LichessError } from '../../src/shared/lichessError'
import { assertMessageText } from '../../src/shared/validate'

const mocks = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  use: vi.fn(),
  getToken: vi.fn(async (): Promise<string | null> => 'Alice-token'),
  readLines: vi.fn(async (_stream: unknown, _onLine: (line: string) => void): Promise<void> => {}),
}))
vi.mock('openapi-fetch', () => ({ default: () => mocks }))
vi.mock('../../src/core/store', () => ({
  loadData: async () => ({ accounts: [{ username: 'Alice', connected: true }] }),
  getToken: mocks.getToken,
}))
vi.mock('../../src/core/usage', () => ({
  meteredFetch: vi.fn(),
  attributeTo: vi.fn(),
  withUsage: (_account: string, _kind: string, fn: () => unknown) => fn(),
}))
vi.mock('../../src/core/ndjson', () => ({ readLines: mocks.readLines }))

beforeEach(() => {
  mocks.GET.mockReset()
  mocks.POST.mockReset().mockResolvedValue({ data: { ok: true } })
  mocks.getToken.mockReset().mockResolvedValue('Alice-token')
  mocks.readLines.mockReset().mockResolvedValue(undefined)
})

it('sends a private message as the chosen account', async () => {
  await expect(sendMessage('Alice', 'Bob', 'Good game!')).resolves.toEqual({ sent: true })
  expect(mocks.POST).toHaveBeenCalledWith(
    '/inbox/{username}',
    expect.objectContaining({
      params: { path: { username: 'Bob' } },
      body: { text: 'Good game!' },
      headers: expect.objectContaining({ Authorization: 'Bearer Alice-token' }),
    }),
  )
})

it('asks to reconnect a login made before messaging was requested', async () => {
  // Lichess refuses a token without the msg:write permission.
  mocks.POST.mockRejectedValue(new LichessError(403, '/inbox/Bob', 'Missing scope'))
  await expect(sendMessage('Alice', 'Bob', 'Hi')).resolves.toEqual({ needsReconnect: true })
  mocks.getToken.mockResolvedValue(null)
  await expect(sendMessage('Alice', 'Bob', 'Hi')).resolves.toEqual({ needsReconnect: true })
})

it('keeps other failures, such as a player who does not accept messages', async () => {
  mocks.POST.mockRejectedValue(new LichessError(400, '/inbox/Bob', 'Cannot send'))
  await expect(sendMessage('Alice', 'Bob', 'Hi')).rejects.toThrow()
})

it('limits message text the way Lichess does', () => {
  expect(assertMessageText('  hello  ')).toBe('hello')
  expect(() => assertMessageText('   ')).toThrow('Type a message first.')
  expect(() => assertMessageText('x'.repeat(8001))).toThrow('8,000')
})

it('reads a player’s recent games from their side, at most ten', async () => {
  mocks.GET.mockResolvedValue({ data: new ReadableStream(), response: new Response() })
  const game = (id: string) =>
    JSON.stringify({
      id,
      rated: true,
      variant: 'standard',
      speed: 'blitz',
      perf: 'blitz',
      createdAt: 1,
      lastMoveAt: 2,
      status: 'mate',
      winner: 'black',
      players: {
        white: { user: { name: 'Carol', id: 'carol' }, rating: 2000 },
        black: { user: { name: 'Bob', id: 'bob' }, rating: 2100, ratingDiff: 6 },
      },
    })
  mocks.readLines.mockImplementation(async (_stream, onLine) => {
    for (let i = 0; i < 12; i++) onLine(game(`game${String(i).padStart(4, '0')}`))
  })
  const games = await recentGames('Bob')
  expect(games).toHaveLength(10)
  expect(games[0]).toMatchObject({
    account: 'Bob',
    color: 'black',
    opponent: 'Carol',
    opponentRating: 2000,
    ratingDiff: 6,
    winner: 'black',
  })
  // Lichess sends a player's games only to signed-in apps, so a connected login goes along.
  expect(mocks.GET).toHaveBeenCalledWith(
    '/api/games/user/{username}',
    expect.objectContaining({
      params: expect.objectContaining({ path: { username: 'Bob' } }),
      headers: expect.objectContaining({ Authorization: 'Bearer Alice-token' }),
    }),
  )
})

it('asks only for permissions Lichess grants to third-party apps', () => {
  // Naming a reserved scope (web:mobile) made Lichess refuse every login with "Invalid scopes".
  expect(OAUTH_SCOPES.filter((scope) => scope.startsWith('web:'))).toEqual([])
  expect(OAUTH_SCOPES).toContain('msg:write')
})
