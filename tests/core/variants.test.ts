import { describe, expect, it } from 'vitest'
import { INITIAL_FEN } from '@kchess/rules/position'
import {
  chess960Fen,
  defaultFen,
  replaySetup,
  STANDARD_SETUP,
  variantFromLichess,
} from '@kchess/rules/variant'
import { setupDrawReason, setupPgn, setupSanHistory } from '@kchess/rules/chess'
import { openingAt, openingKey, openingReplies, openingTree } from '@kchess/rules/openings'

describe('variants', () => {
  it('numbers Chess960 starts as the Scharnagl scheme does', () => {
    expect(chess960Fen(518)).toBe(INITIAL_FEN)
    expect(chess960Fen(0).split(' ')[0]!.split('/')[0]).toBe('bbqnnrkr')
    expect(chess960Fen(959).split(' ')[0]!.split('/')[0]).toBe('rkrnnqbb')
    // Every start has the king between the rooks and bishops on opposite colours.
    for (let n = 0; n < 960; n += 37) {
      const rank = chess960Fen(n).split('/')[0]!
      expect(rank.indexOf('r')).toBeLessThan(rank.indexOf('k'))
      expect(rank.lastIndexOf('r')).toBeGreaterThan(rank.indexOf('k'))
      expect(rank.indexOf('b') % 2).not.toBe(rank.lastIndexOf('b') % 2)
    }
  })

  it('replays castling written either way, and stops at an illegal move', () => {
    const moves = ['e2e4', 'e7e5', 'g1f3', 'b8c6', 'f1c4', 'g8f6']
    for (const castle of ['e1g1', 'e1h1'])
      expect(replaySetup(STANDARD_SETUP, [...moves, castle])?.played.at(-1)?.san).toBe('O-O')
    expect(
      replaySetup({ variant: 'chess960', fen: INITIAL_FEN }, [...moves, 'e1h1'])?.played.at(-1)
        ?.san,
    ).toBe('O-O')
    expect(replaySetup(STANDARD_SETUP, ['e2e4', 'e2e4'])?.played).toHaveLength(1)
    // A list edited in place, keeping its length and its last two moves, is replayed again.
    const edited = ['e2e4', 'e7e5', 'g1f3', 'b8c6']
    expect(setupSanHistory(STANDARD_SETUP, edited)[0]).toBe('e4')
    edited[0] = 'd2d4'
    expect(setupSanHistory(STANDARD_SETUP, edited)[0]).toBe('d4')
  })

  it('knows each variant’s start and rules', () => {
    expect(defaultFen('horde').split(' ')[0]).toContain('PPPPPPPP/PPPPPPPP')
    expect(defaultFen('racingKings')).toBe('8/8/8/8/8/8/krbnNBRK/qrbnNBRQ w - - 0 1')
    expect(variantFromLichess('fromPosition')).toBe('standard')
    expect(variantFromLichess('crazyhouse')).toBeUndefined()
    // King of the Hill: a king on the centre wins at once.
    const hill = replaySetup({ variant: 'kingOfTheHill', fen: '8/8/8/8/8/3K4/8/k7 w - - 0 1' }, [
      'd3d4',
    ])
    expect(hill?.position.isVariantEnd()).toBe(true)
    // Antichess: captures are compulsory.
    const anti = replaySetup({ variant: 'antichess', fen: defaultFen('antichess') }, [
      'e2e4',
      'd7d5',
      'a2a3',
    ])
    expect(anti?.played).toHaveLength(2)
  })

  it('names the variant and start in PGN, and counts repetition from the start', () => {
    const pgn = setupPgn({ variant: 'chess960', fen: chess960Fen(0) }, ['e2e4'], { White: 'A' })
    expect(pgn).toContain('[Variant "Chess960"]')
    expect(pgn).toContain(`[FEN "${chess960Fen(0)}"]`)
    expect(pgn).toContain('[White "A"]')
    const shuffle = ['g1f3', 'g8f6', 'f3g1', 'f6g8', 'g1f3', 'g8f6', 'f3g1', 'f6g8']
    expect(setupDrawReason(STANDARD_SETUP, shuffle)).toBe('Threefold repetition')
    expect(setupDrawReason({ variant: 'antichess', fen: defaultFen('antichess') }, shuffle)).toBe(
      undefined,
    )
    expect(setupSanHistory(STANDARD_SETUP, ['e2e4', 'e7e5'])).toEqual(['e4', 'e5'])
  })

  it('names openings by stepping back to the last named position', () => {
    expect(openingAt(STANDARD_SETUP, ['e2e4', 'a7a6', 'h2h3'])).toEqual({
      eco: 'B00',
      name: 'St. George Defense',
    })
    expect(openingAt(STANDARD_SETUP, ['e2e4'])).toEqual({
      eco: 'B00',
      name: "King's Pawn Game",
    })
    expect(openingAt(STANDARD_SETUP, ['d2d4'])).toEqual({
      eco: 'A40',
      name: "Queen's Pawn Game",
    })
    expect(openingAt({ variant: 'chess960', fen: INITIAL_FEN }, ['e2e4'])).toBeUndefined()
  })

  it('browses openings by family and picks the replies a drill continues with', () => {
    const gambit = openingTree().find((family) => family.name === "King's Gambit")
    expect(gambit).toMatchObject({ eco: 'C30', moves: ['e2e4', 'e7e5', 'f2f4'] })
    const accepted = gambit?.children.find((child) => child.label === 'Accepted')
    expect(accepted?.name).toBe("King's Gambit Accepted")
    // Every named line opens on the position the name is given to.
    for (const node of accepted?.children ?? [])
      if (node.moves) expect(openingAt(STANDARD_SETUP, node.moves)?.name).toBe(node.name)
    const stats = [
      { uci: 'e5f4', white: 45, draws: 5, black: 45 },
      { uci: 'f8c5', white: 2, draws: 0, black: 3 },
      { uci: 'd7d5', white: 10, draws: 2, black: 8 },
    ]
    expect(openingReplies(stats, true)).toEqual(['e5f4'])
    expect(openingReplies(stats, false)).toEqual(['e5f4', 'd7d5'])
    expect(openingKey("King's Gambit Accepted: Kieseritzky Gambit, Long Whip")).toBe(
      'Kings_Gambit_Accepted_Kieseritzky_Gambit',
    )
  })
})
