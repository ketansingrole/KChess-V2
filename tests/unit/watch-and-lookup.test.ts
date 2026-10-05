import { describe, expect, it, vi } from 'vitest'
import { INITIAL_FEN } from 'chessops/fen'
import { broadcastGame, displayFen } from '../../src/main/spectate'
import { splitPgn } from '../../src/main/studies'
import { PositionLookupService } from '../../src/main/positionLookup'
import type { PositionLookup } from '../../src/shared/types'
import { assertExport, assertOnlineOptions, assertWatchTarget } from '../../src/shared/validate'

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
