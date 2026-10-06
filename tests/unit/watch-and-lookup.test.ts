import { describe, expect, it, vi } from 'vitest'
import { client } from '../../src/main/lichess'
import { INITIAL_FEN, makeFen } from 'chessops/fen'
import {
  alignTvMoves,
  broadcastGame,
  broadcastSummaries,
  broadcasts,
  displayFen,
  parseGameDetails,
  parseTvGame,
} from '../../src/main/spectate'
import { STANDARD_SETUP, replaySetup } from '../../src/shared/variant'
import { materialBalance } from '../../app/utils/material'
import { splitPgn } from '../../src/main/studies'
import { PositionLookupService } from '../../src/main/positionLookup'
import type { PositionLookup } from '../../src/shared/types'
import {
  assertBroadcastQuery,
  assertExport,
  assertOnlineOptions,
  assertWatchTarget,
} from '../../src/shared/validate'

vi.mock('../../src/main/usage', () => ({
  withUsage: (_account: string, _kind: string, work: () => unknown) => work(),
}))

vi.mock('electron', () => ({ shell: {} }))
vi.mock('../../src/main/store', () => ({ getToken: vi.fn() }))

const pgn = `[Event "Test Open"]
[White "Alpha"]
[Black "Beta"]
[WhiteElo "2500"]
[WhiteTitle "GM"]
[Result "*"]
[GameURL "https://lichess.org/broadcast/test/round-1/Nhw57KE3/2AJYiCJG"]

1. e4 { [%clk 0:30:00] } 1... e5 { [%clk 0:29:40] } 2. Nf3 { [%eval 0.2] [%clk 0:29:10] } *`

describe('watching', () => {
  it('searches recent official broadcasts rather than filtering only featured events', async () => {
    const get = vi.spyOn(client, 'GET').mockResolvedValue({
      data: {
        currentPageResults: [
          {
            tour: { id: 'search01', name: 'Historical Open' },
            round: { id: 'round001', name: 'Final', finishedAt: 1700000000000 },
          },
        ],
      },
      response: new Response(),
    } as never)
    expect(await broadcasts(' Historical ')).toMatchObject([
      { tourName: 'Historical Open', section: 'past' },
    ])
    expect(get).toHaveBeenCalledWith('/api/broadcast/search', {
      params: { query: { q: 'Historical', page: 1 } },
    })
    expect(assertBroadcastQuery(undefined)).toBeUndefined()
    expect(assertBroadcastQuery(' Open ')).toBe('Open')
    expect(() => assertBroadcastQuery('x'.repeat(101))).toThrow()
  })

  it('classifies scheduled rounds correctly and retains trusted broadcast images', () => {
    const entries = [
      {
        tour: { id: 'tour1234', name: 'Open', image: 'https://image.lichess1.org/example.webp' },
        round: { id: 'round123', name: 'Round 4', startsAt: 1800000000000 },
      },
      {
        tour: { id: 'tour5678', name: 'Live', image: 'https://untrusted.example/image.jpg' },
        round: { id: 'round456', name: 'Round 1', ongoing: true },
      },
      {
        tour: { id: 'tour9012', name: 'Finished' },
        round: { id: 'round789', name: 'Final', finished: true },
      },
    ]
    const [scheduled, live, past] = broadcastSummaries(entries, 'past')
    expect(scheduled).toMatchObject({
      section: 'upcoming',
      startsAt: 1800000000000,
      image: 'https://image.lichess1.org/example.webp',
    })
    expect(live).toMatchObject({ section: 'active', ongoing: true })
    expect(live.image).toBeUndefined()
    expect(past.section).toBe('past')
    expect(broadcastSummaries([{ tour: { id: 123 } }], 'active')).toEqual([])
  })

  it('reads a broadcast board: players, clocks, moves and its chapter id', () => {
    const game = broadcastGame(pgn)!
    expect(game).toMatchObject({
      id: '2AJYiCJG',
      white: { name: 'Alpha', title: 'GM', rating: 2500 },
      black: { name: 'Beta' },
      moves: ['e2e4', 'e7e5', 'g1f3'],
      whiteClock: 1750,
      blackClock: 1780,
      ongoing: true,
      startFen: INITIAL_FEN,
    })
    expect(broadcastGame('not a game')?.moves ?? []).toEqual([])
  })

  it('keeps only a board and side to move from a streamed FEN', () => {
    expect(displayFen('rnbqkb1r/ppp1pppp/5n2/3p4/3P4/2N5/PPP1PPPP/R1BQKBNR[] w KQkq - 2 3')).toBe(
      'rnbqkb1r/ppp1pppp/5n2/3p4/3P4/2N5/PPP1PPPP/R1BQKBNR w KQkq - 0 1',
    )
    expect(displayFen('rnbqkbnr/pppppppp b')).toBeUndefined()
    expect(displayFen('8/8/8/8/8/8/8/8<script> w - - 0 1')).toBeUndefined()
  })

  it('splits a study into chapters', () => {
    expect(splitPgn(`${pgn}\n\n\n${pgn.replace('Test Open', 'Second')}\n`)).toHaveLength(2)
  })
})

describe('validation of the new requests', () => {
  it('accepts only known TV channels or game ids', () => {
    expect(assertWatchTarget({ channel: 'rapid' }, ['rapid'])).toEqual({ channel: 'rapid' })
    expect(assertWatchTarget({ gameId: 'AbCd1234' }, ['rapid'])).toEqual({ gameId: 'AbCd1234' })
    expect(() => assertWatchTarget({ channel: '../x' }, ['rapid'])).toThrow()
  })
  it('checks that exported bytes are the kind of file they claim', () => {
    const gif = new Uint8Array([71, 73, 70, 56, 57, 97])
    expect(assertExport({ name: 'A vs B', kind: 'gif', data: gif }).kind).toBe('gif')
    expect(() => assertExport({ name: 'A vs B', kind: 'png', data: gif })).toThrow()
    expect(() => assertExport({ name: '../../etc/passwd', kind: 'pgn', data: '1. e4' })).toThrow()
  })
  it('accepts correspondence days and variants for online games', () => {
    expect(
      assertOnlineOptions({
        minutes: 10,
        increment: 0,
        color: 'random',
        rated: false,
        days: 3,
        variant: 'atomic',
      }),
    ).toMatchObject({ days: 3, variant: 'atomic' })
    expect(() =>
      assertOnlineOptions({ minutes: 10, increment: 0, color: 'random', rated: false, days: 4 }),
    ).toThrow()
    expect(() =>
      assertOnlineOptions({
        minutes: 10,
        increment: 0,
        color: 'random',
        rated: false,
        variant: 'crazyhouse',
      }),
    ).toThrow()
  })
})

function service(fetcher: typeof fetch) {
  const values = new Map<string, PositionLookup>()
  const lookups = new PositionLookupService(
    {
      read: (key) => values.get(key),
      write: (key, value) => values.set(key, { ...value, cached: false, stale: false }),
    },
    fetcher,
    () => 1_000,
  )
  return { lookups, values }
}

describe('opening explorer databases', () => {
  it('asks the masters database and lists its top games', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      Response.json({
        white: 10,
        draws: 5,
        black: 5,
        moves: [{ uci: 'e2e4', white: 10, draws: 5, black: 5 }],
        topGames: [
          {
            id: 'AbCdEfGh',
            winner: 'white',
            white: { name: 'Carlsen', rating: 2850 },
            black: { name: 'Caruana', rating: 2800 },
            year: 2019,
            uci: 'e2e4',
          },
        ],
      }),
    )
    const { lookups } = service(fetcher)
    const result = await lookups.lookup('masters', INITIAL_FEN, { since: '2000-01' })
    expect(String(fetcher.mock.calls[0]![0])).toContain('explorer.lichess.org/masters')
    expect(String(fetcher.mock.calls[0]![0])).toContain('since=2000')
    expect(result.total).toBe(20)
    expect(result.games?.[0]).toMatchObject({ white: 'Carlsen', san: 'e4', winner: 'white' })
  })

  it('reads the last snapshot of the player database and keys its cache by filters', async () => {
    const snapshot = (n: number) =>
      JSON.stringify({ moves: [{ uci: 'd2d4', white: n, draws: 0, black: 0 }] })
    const fetcher = vi
      .fn<typeof fetch>()
      .mockImplementation(async () => new Response(`${snapshot(1)}\n${snapshot(7)}\n`))
    const { lookups, values } = service(fetcher)
    const result = await lookups.lookup('player', INITIAL_FEN, { player: 'Alice', color: 'black' })
    expect(result.moves[0]?.white).toBe(7)
    const url = String(fetcher.mock.calls[0]![0])
    expect(url).toContain('player=Alice')
    expect(url).toContain('color=black')
    await lookups.lookup('player', INITIAL_FEN, { player: 'Bob', color: 'black' })
    expect(fetcher).toHaveBeenCalledTimes(2)
    expect(values.size).toBe(2)
    await expect(lookups.lookup('player', INITIAL_FEN, {})).rejects.toThrow('whose games')
  })

  it('rejects unknown filters', async () => {
    const { lookups } = service(vi.fn<typeof fetch>())
    await expect(lookups.lookup('opening', INITIAL_FEN, { ratings: [1234] })).rejects.toThrow()
  })
})

describe('TV game details', () => {
  it('keeps the time control and rated flag the TV feed leaves out', () => {
    expect(
      parseGameDetails({
        id: 'abcdefgh',
        rated: true,
        speed: 'bullet',
        clock: { initial: 60, increment: 0, totalTime: 60 },
      }),
    ).toEqual({ rated: true, speed: 'bullet', clock: { initial: 60, increment: 0 } })
    expect(parseGameDetails({ rated: false, speed: 'correspondence' })).toEqual({
      rated: false,
      speed: 'correspondence',
      clock: undefined,
    })
    expect(parseGameDetails({ rated: 'yes' })).toEqual({})
  })

  it('counts material in pawns, positive when White is ahead', () => {
    expect(materialBalance(INITIAL_FEN)).toBe(0)
    // A rook against a bishop shows as +2, as on Lichess TV.
    expect(materialBalance('4k3/8/8/8/8/2b5/8/R3K3 w - - 0 1')).toBe(2)
    expect(materialBalance('4k3/8/8/8/8/8/8/3QK3 b - - 0 1')).toBe(9)
    expect(materialBalance('3qk3/8/8/8/8/8/8/4K3 w - - 0 1')).toBe(-9)
  })
})

describe('TV move list', () => {
  // e4 e5 Nf3 Nc6 Bc4 Bc5, with the feed's view of each position.
  const uci = ['e2e4', 'e7e5', 'g1f3', 'b8c6', 'f1c4', 'f8c5']
  const san = ['e4', 'e5', 'Nf3', 'Nc6', 'Bc4', 'Bc5']
  const key = (ply: number) =>
    makeFen(replaySetup(STANDARD_SETUP, uci.slice(0, ply))!.position.toSetup())
      .split(' ')
      .slice(0, 2)
      .join(' ')

  it('joins a lagging export to the moves the feed sent after it', () => {
    // The feed joined after Nf3 and has since seen Nc6, Bc4 and Bc5; the export stops at Nc6.
    const feed = [key(3), key(4), key(5), key(6)]
    const live = ['b8c6', 'f1c4', 'f8c5']
    expect(alignTvMoves(STANDARD_SETUP, san.slice(0, 4), feed, live)).toEqual(uci)
  })

  it('waits while the export has not reached anything the feed showed', () => {
    const feed = [key(5), key(6)]
    expect(alignTvMoves(STANDARD_SETUP, san.slice(0, 3), feed, ['f8c5'])).toBeUndefined()
  })

  it('refuses a list with a gap or that ends somewhere else than the board', () => {
    const feed = [key(3), key(4), key(5)]
    expect(alignTvMoves(STANDARD_SETUP, san.slice(0, 3), feed, ['b8c6', undefined])).toBeUndefined()
    expect(alignTvMoves(STANDARD_SETUP, san.slice(0, 3), feed, ['b8c6', 'f1b5'])).toBeUndefined()
  })

  it('reads the start, moves and result from a game export', () => {
    expect(
      parseTvGame({
        variant: 'chess960',
        initialFen: 'bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w HFhf - 0 1',
        moves: 'e4 e5',
        status: 'mate',
        winner: 'black',
        rated: true,
      }),
    ).toMatchObject({
      setup: {
        variant: 'chess960',
        fen: 'bbqnnrkr/pppppppp/8/8/8/8/PPPPPPPP/BBQNNRKR w HFhf - 0 1',
      },
      san: ['e4', 'e5'],
      status: 'mate',
      winner: 'black',
      details: { rated: true },
    })
    // Crazyhouse cannot be replayed, so it gets the board without a move list.
    expect(parseTvGame({ variant: 'crazyhouse', moves: 'e4' }).setup).toBeUndefined()
  })
})
