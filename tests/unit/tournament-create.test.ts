import { beforeEach, expect, it, vi } from 'vitest'
import { arenaExtras, createTournament, tournaments } from '../../src/core/tournaments'
import { LichessError } from '../../src/shared/lichessError'
import { assertNewArena } from '../../src/shared/validate'

const mocks = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  use: vi.fn(),
  getToken: vi.fn(async (): Promise<string | null> => 'Alice-token'),
}))
vi.mock('openapi-fetch', () => ({ default: () => mocks }))
vi.mock('../../src/core/store', () => ({
  loadData: async () => ({ accounts: [] }),
  getToken: mocks.getToken,
}))
vi.mock('../../src/core/usage', () => ({
  meteredFetch: vi.fn(),
  attributeTo: vi.fn(),
  withUsage: (_account: string, _kind: string, fn: () => unknown) => fn(),
}))

const arena = (overrides: Record<string, unknown> = {}) => ({
  id: 'Abcd1234',
  fullName: '≤1700 Rapid Arena',
  status: 10,
  variant: { key: 'standard', name: 'Standard' },
  rated: true,
  clock: { limit: 600, increment: 0 },
  minutes: 57,
  nbPlayers: 12,
  startsAt: 1_791_259_200_000,
  finishesAt: 1_791_262_620_000,
  perf: { key: 'rapid' },
  schedule: { freq: 'hourly', speed: 'rapid' },
  maxRating: { rating: 1700 },
  ...overrides,
})

beforeEach(() => {
  mocks.GET.mockReset()
  mocks.POST.mockReset()
  mocks.getToken.mockReset().mockResolvedValue('Alice-token')
})

it('lists running, upcoming and finished arenas with what the timeline groups them by', async () => {
  mocks.GET.mockResolvedValue({
    data: {
      started: [arena({ id: 'Started1', status: 20 })],
      created: [arena()],
      finished: [arena({ id: 'Finish01', status: 30, maxRating: null, schedule: null })],
    },
  })
  const list = await tournaments('')
  expect(list.arenas.map((t) => [t.id, t.status])).toEqual([
    ['Started1', 'started'],
    ['Abcd1234', 'created'],
    ['Finish01', 'finished'],
  ])
  expect(list.arenas[1]).toMatchObject({ perf: 'rapid', freq: 'hourly', maxRating: 1700 })
  expect(list.arenas[2]).toMatchObject({ freq: undefined, maxRating: undefined })
})

it('creates an arena as the chosen account and returns it', async () => {
  mocks.POST.mockResolvedValue({ data: arena({ fullName: 'Club Night Arena', maxRating: null }) })
  const created = await createTournament('Alice', {
    name: 'Club Night',
    clockTime: 10,
    clockIncrement: 0,
    minutes: 60,
    waitMinutes: 5,
    variant: 'standard',
    rated: true,
  })
  expect(created).toMatchObject({ id: 'Abcd1234', name: 'Club Night Arena', playable: true })
  expect(mocks.POST).toHaveBeenCalledWith(
    '/api/tournament',
    expect.objectContaining({
      body: expect.objectContaining({
        name: 'Club Night',
        clockTime: 10,
        minutes: 60,
        waitMinutes: 5,
      }),
      headers: expect.objectContaining({ Authorization: 'Bearer Alice-token' }),
    }),
  )
})

it('asks to reconnect when the login lacks the tournament permission', async () => {
  mocks.POST.mockRejectedValue(new LichessError(403, '/api/tournament', 'Missing scope'))
  await expect(
    createTournament('Alice', {
      clockTime: 3,
      clockIncrement: 2,
      minutes: 60,
      variant: 'standard',
      rated: true,
    }),
  ).resolves.toEqual({ needsReconnect: true })
})

it('accepts only the clocks, durations and variants Lichess allows', () => {
  const valid = { clockTime: 3, clockIncrement: 2, minutes: 60, variant: 'atomic', rated: false }
  expect(assertNewArena(valid)).toEqual(valid)
  expect(() => assertNewArena({ ...valid, clockTime: 9 })).toThrow('Invalid clock.')
  expect(() => assertNewArena({ ...valid, minutes: 61 })).toThrow('Invalid duration.')
  expect(() => assertNewArena({ ...valid, variant: 'fromPosition' })).toThrow('Invalid variant.')
  expect(() => assertNewArena({ ...valid, name: 'x'.repeat(31) })).toThrow('30 characters')
  expect(() => assertNewArena({ ...valid, extra: true })).toThrow()
})

it('looks up the account’s teams signed in, and explains a refusal briefly', async () => {
  // Lichess answers /api/team/of only with a login ("Missing authorization header").
  mocks.GET.mockImplementation(async (path: string) => {
    if (path === '/api/tournament') return { data: { started: [], created: [], finished: [] } }
    if (path === '/api/team/of/{username}')
      throw new LichessError(401, path, 'Missing authorization header')
    throw new Error(`unexpected ${path}`)
  })
  const list = await tournaments('Alice')
  expect(mocks.GET).toHaveBeenCalledWith(
    '/api/team/of/{username}',
    expect.objectContaining({ headers: { Authorization: 'Bearer Alice-token' } }),
  )
  expect(list.problems).toEqual([
    'Couldn’t load the teams of @Alice. Connect the account again to allow it.',
  ])
  expect(list.problems.join(' ')).not.toContain('authorization header')
})

it('reads an arena’s top game, games in progress, podium and totals', () => {
  const extras = arenaExtras({
    featured: {
      id: 'ShLmo1Hs',
      fen: '8/5rpp/4R3/6KP/3k4/5qP1/8/8 w',
      orientation: 'black',
      lastMove: 'd1f3',
      white: { name: 'weaky_player', rating: 2482, rank: 3 },
      black: { name: 'Coach13', rating: 2868, rank: 1 },
      c: { white: 4, black: 10 },
    },
    duels: [
      {
        id: 'ShLmo1Hs',
        p: [
          { n: 'weaky_player', r: 2482, k: 3 },
          { n: 'Coach13', r: 2868, k: 1 },
        ],
      },
    ],
    podium: [{ name: 'agill47', rank: 1, rating: 1262, score: 8, performance: 1637 }],
    stats: {
      games: 8,
      moves: 378,
      whiteWins: 3,
      blackWins: 5,
      draws: 0,
      berserks: 0,
      averageRating: 1132,
    },
  })
  expect(extras.featured).toMatchObject({
    fen: '8/5rpp/4R3/6KP/3k4/5qP1/8/8 w - - 0 1',
    orientation: 'black',
    black: { name: 'Coach13', rank: 1 },
    clocks: { white: 4, black: 10 },
  })
  expect(extras.duels).toEqual([
    {
      id: 'ShLmo1Hs',
      white: { name: 'weaky_player', rating: 2482, rank: 3 },
      black: { name: 'Coach13', rating: 2868, rank: 1 },
    },
  ])
  expect(extras.podium?.[0]).toMatchObject({ name: 'agill47', performance: 1637 })
  expect(extras.stats).toMatchObject({ games: 8, blackWins: 5 })
  // Nothing readable: no extras rather than a broken event page.
  expect(arenaExtras({ duels: [{ id: 1 }] })).toEqual({})
})
