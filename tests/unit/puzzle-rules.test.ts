import { describe, expect, it } from 'vitest'
import {
  moveSquares,
  nextSolutionSquares,
  opponentReply,
  playerColor,
  playerMove,
  puzzleFromApi,
  puzzleFromDb,
  startPuzzle,
  type ApiPuzzle,
  type DbPuzzle,
} from '@kchess/rules/puzzle'
import type { Puzzle } from '@kchess/contracts/types'

const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'

/** A Lichess daily puzzle: White to play f4 stepping into a winning line. */
const DAILY: ApiPuzzle = {
  game: {
    id: 'qLcxVHzB',
    pgn: 'd4 Nf6 c4 d6 Nc3 g6 e4 Bg7 Be2 O-O e5 dxe5 dxe5 Qxd1+ Nxd1 Ne4 Be3 Bxe5 Bd3 Nf6',
  },
  puzzle: {
    id: '9862H',
    rating: 1850,
    plays: 53523,
    solution: ['f2f4', 'e5d6', 'c4c5'],
    themes: ['opening', 'advantage'],
    fen: 'rnb2rk1/ppp1pp1p/5np1/4b3/2P5/3BB3/PP3PPP/R2NK1NR w KQ - 1 1',
    lastMove: 'e4f6',
    initialPly: 19,
  },
}

const DB_ROW: DbPuzzle = {
  id: '00sHx',
  fen: 'q3k1nr/1pp1nQpp/3p4/1P2p3/4P3/B1PP1b2/B5PP/5K2 b k - 0 17',
  moves: 'e8d7 a2e6 d7d8 f7f8',
  rating: 1760,
  plays: 550,
  themes: 'mate mateIn2 middlegame short',
}

const MATE_IN_ONE: Puzzle = {
  id: 'm1',
  fen: '6k1/5ppp/8/8/8/8/5PPP/R5K1 w - - 0 1',
  solution: ['a1a8'],
  rating: 600,
  themes: [],
}

describe('puzzleFromApi', () => {
  it('reduces a Lichess API puzzle to one shape', () => {
    expect(puzzleFromApi(DAILY)).toEqual({
      id: '9862H',
      fen: 'rnb2rk1/ppp1pp1p/5np1/4b3/2P5/3BB3/PP3PPP/R2NK1NR w KQ - 1 1',
      lastMove: 'e4f6',
      solution: ['f2f4', 'e5d6', 'c4c5'],
      rating: 1850,
      themes: ['opening', 'advantage'],
      plays: 53523,
      gameId: 'qLcxVHzB',
    })
  })

  it('returns undefined when no puzzle can be built', () => {
    expect(
      puzzleFromApi({
        game: {},
        puzzle: { id: 'x', initialPly: 0, rating: 1500, solution: [], themes: [] },
      }),
    ).toBeUndefined()
  })
})

describe('puzzleFromDb', () => {
  it('plays the opening move of a database row into the puzzle', () => {
    expect(puzzleFromDb(DB_ROW)).toEqual({
      id: '00sHx',
      fen: 'q5nr/1ppknQpp/3p4/1P2p3/4P3/B1PP1b2/B5PP/5K2 w - - 1 18',
      lastMove: 'e8d7',
      solution: ['a2e6', 'd7d8', 'f7f8'],
      rating: 1760,
      themes: ['mate', 'mateIn2', 'middlegame', 'short'],
      plays: 550,
    })
  })

  it('returns undefined for a row that does not parse', () => {
    expect(puzzleFromDb({ ...DB_ROW, fen: 'nonsense' })).toBeUndefined()
    expect(puzzleFromDb({ ...DB_ROW, moves: 'a1a8 a2e6' })).toBeUndefined()
  })
})

describe('moveSquares', () => {
  it('splits a UCI move into its squares', () => {
    expect(moveSquares(START, 'e2e4')).toEqual(['e2', 'e4'])
  })

  it('shows castling as the king’s two-square step', () => {
    const home = 'r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1'
    expect(moveSquares(home, 'e1g1')).toEqual(['e1', 'g1'])
    expect(moveSquares(home, 'e1c1')).toEqual(['e1', 'c1'])
  })

  it('returns undefined when the move does not parse', () => {
    expect(moveSquares(START, 'bogus')).toBeUndefined()
    expect(moveSquares(START, '')).toBeUndefined()
  })
})

describe('solving', () => {
  it('starts at the first solution move with the offered move highlighted', () => {
    const puzzle = puzzleFromApi(DAILY)
    expect(puzzle).toBeDefined()
    const state = startPuzzle(puzzle as Puzzle)
    expect(state).toMatchObject({ index: 0, status: 'playing', sans: [] })
    expect(state.fen).toBe((puzzle as Puzzle).fen)
    expect(state.lastMove).toEqual(['e4', 'f6'])
    expect(playerColor(puzzle as Puzzle)).toBe('white')
    expect(nextSolutionSquares(puzzle as Puzzle, state)).toEqual(['f2', 'f4'])
  })

  it('advances through the solution with the opponent’s replies', () => {
    const puzzle = puzzleFromApi(DAILY) as Puzzle
    const state = startPuzzle(puzzle)
    const first = playerMove(puzzle, state, 'f2f4')
    expect(first.correct).toBe(true)
    expect(first.reply).toBe('e5d6')
    expect(first.state).toMatchObject({ index: 1, status: 'playing', sans: ['f4'] })
    const continued = opponentReply(first.state, first.reply as string)
    expect(continued).toMatchObject({ index: 2, status: 'playing', sans: ['f4', 'Bd6'] })
    expect(nextSolutionSquares(puzzle, continued)).toEqual(['c4', 'c5'])
  })

  it('leaves the state alone on a wrong move', () => {
    const puzzle = puzzleFromApi(DAILY) as Puzzle
    const state = startPuzzle(puzzle)
    const wrong = playerMove(puzzle, state, 'g1f3')
    expect(wrong.correct).toBe(false)
    expect(wrong.reply).toBeUndefined()
    expect(wrong.state).toMatchObject({ index: 0, status: 'playing', sans: [] })
    expect(wrong.state.fen).toBe(state.fen)
  })

  it('solves a mate in one at once', () => {
    const state = startPuzzle(MATE_IN_ONE)
    const solved = playerMove(MATE_IN_ONE, state, 'a1a8')
    expect(solved.correct).toBe(true)
    expect(solved.reply).toBeUndefined()
    expect(solved.state).toMatchObject({ index: 1, status: 'solved', sans: ['Ra8#'] })
    expect(playerColor(MATE_IN_ONE)).toBe('white')
    expect(nextSolutionSquares(MATE_IN_ONE, solved.state)).toBeUndefined()
  })
})
