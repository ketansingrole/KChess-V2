/** Turning puzzles from Lichess (API or database) into one shape, and the move rules of solving them. */
import { Chess, castlingSide, normalizeMove, type Position } from 'chessops/chess'
import { makeFen, parseFen } from 'chessops/fen'
import { makeSanAndPlay, parseSan } from 'chessops/san'
import { makeSquare, makeUci, parseUci, squareRank } from 'chessops/util'
import type { Puzzle } from './types.ts'

/** The JSON Lichess returns for a puzzle (`PuzzleAndGame`), reduced to what is read. */
export interface ApiPuzzle {
  game: { id?: string; pgn?: string }
  puzzle: {
    id: string
    initialPly: number
    plays?: number
    rating: number
    fen?: string
    lastMove?: string
    solution: string[]
    themes: string[]
  }
}

function positionFromFen(fen: string): Position | undefined {
  const setup = parseFen(fen)
  if (setup.isErr) return undefined
  const pos = Chess.fromSetup(setup.value)
  return pos.isOk ? pos.value : undefined
}

/**
 * From a Lichess API puzzle. Current responses carry the starting `fen` and `lastMove`; older ones
 * only the game's moves, of which the first `initialPly + 1` lead to the puzzle.
 */
export function puzzleFromApi(raw: ApiPuzzle): Puzzle | undefined {
  const { puzzle, game } = raw
  let fen = puzzle.fen
  let lastMove = puzzle.lastMove
  if (!fen || !lastMove) {
    const pos = Chess.default()
    const sans = (game.pgn ?? '').split(/\s+/).filter((token) => token && !/^\d+\.+$/.test(token))
    for (const san of sans.slice(0, puzzle.initialPly + 1)) {
      const move = parseSan(pos, san)
      if (!move) return undefined
      lastMove = makeUci(move)
      pos.play(move)
    }
    fen = makeFen(pos.toSetup())
  }
  if (!puzzle.solution?.length) return undefined
  return {
    id: puzzle.id,
    fen,
    lastMove,
    solution: [...puzzle.solution],
    rating: puzzle.rating,
    themes: [...puzzle.themes],
    plays: puzzle.plays,
    gameId: game.id,
  }
}

/** A row of Lichess's public puzzle database: its FEN is *before* the opponent's move, which opens `moves`. */
export interface DbPuzzle {
  id: string
  fen: string
  moves: string
  rating: number
  plays?: number
  themes: string
}

export function puzzleFromDb(row: DbPuzzle): Puzzle | undefined {
  const [first, ...solution] = row.moves.split(' ').filter(Boolean)
  const pos = positionFromFen(row.fen)
  const opening = first ? parseUci(first) : undefined
  if (!pos || !opening || !solution.length || !pos.isLegal(opening)) return undefined
  pos.play(opening)
  return {
    id: row.id,
    fen: makeFen(pos.toSetup()),
    lastMove: first,
    solution,
    rating: row.rating,
    themes: row.themes.split(' ').filter(Boolean),
    plays: row.plays,
  }
}

/** Squares to highlight for a UCI move; castling shows the king's two-square step, not "king takes rook". */
export function moveSquares(fen: string, uci: string): [string, string] | undefined {
  const move = parseUci(uci)
  if (!move || !('from' in move)) return undefined
  const pos = positionFromFen(fen)
  const side = pos && castlingSide(pos, move)
  if (!side) return [makeSquare(move.from), makeSquare(move.to)]
  const rank = squareRank(move.from) + 1
  return [makeSquare(move.from), `${side === 'h' ? 'g' : 'c'}${rank}`]
}

export type PuzzleStatus = 'playing' | 'solved' | 'failed'

export interface PuzzleState {
  fen: string
  /** Index in `solution` of the move the player must make next. */
  index: number
  status: PuzzleStatus
  /** Squares of the last move played, for the board. */
  lastMove?: string[]
  /** SAN of every move played in the puzzle so far, opponent's first. */
  sans: string[]
}

export function startPuzzle(puzzle: Puzzle): PuzzleState {
  return {
    fen: puzzle.fen,
    index: 0,
    status: 'playing',
    lastMove: puzzle.lastMove ? moveSquares(puzzle.fen, puzzle.lastMove) : undefined,
    sans: [],
  }
}

function sameMove(pos: Position, a: string, b: string): boolean {
  const first = parseUci(a)
  const second = parseUci(b)
  if (!first || !second) return false
  return makeUci(normalizeMove(pos, first)) === makeUci(normalizeMove(pos, second))
}

/** Play `uci` from `state`; returns the new state, or undefined when the move is illegal. */
function applyMove(state: PuzzleState, uci: string): PuzzleState | undefined {
  const pos = positionFromFen(state.fen)
  const move = parseUci(uci)
  if (!pos || !move || !pos.isLegal(move)) return undefined
  const squares = moveSquares(state.fen, uci)
  const san = makeSanAndPlay(pos, move)
  return { ...state, fen: makeFen(pos.toSetup()), lastMove: squares, sans: [...state.sans, san] }
}

export interface PlayerMove {
  /** The state after the player's move; unchanged for a wrong move. */
  state: PuzzleState
  correct: boolean
  /** The opponent's reply to play next (see `opponentReply`), when the puzzle goes on. */
  reply?: string
}

/**
 * The player moves. The solution's move is right, and so is any other move that checkmates
 * (Lichess accepts those too). The puzzle is solved once the last solution move — or a mate — is played.
 */
export function playerMove(puzzle: Puzzle, state: PuzzleState, uci: string): PlayerMove {
  const pos = positionFromFen(state.fen)
  const expected = puzzle.solution[state.index]
  if (state.status !== 'playing' || !pos || !expected) return { state, correct: false }
  const move = parseUci(uci)
  if (!move || !pos.isLegal(move)) return { state, correct: false }
  const after = pos.clone()
  after.play(move)
  const mates = after.isCheckmate()
  if (!sameMove(pos, uci, expected) && !mates) return { state, correct: false }
  const next = applyMove(state, uci)
  if (!next) return { state, correct: false }
  const index = state.index + 1
  if (mates || index >= puzzle.solution.length)
    return { state: { ...next, index, status: 'solved' }, correct: true }
  return { state: { ...next, index }, correct: true, reply: puzzle.solution[index] }
}

/** Play the opponent's scripted reply. */
export function opponentReply(state: PuzzleState, uci: string): PuzzleState {
  const next = applyMove(state, uci)
  return next ? { ...next, index: state.index + 1 } : state
}

/** Squares of the next solution move, for a hint (`from`) or the solution arrow. */
export function nextSolutionSquares(
  puzzle: Puzzle,
  state: PuzzleState,
): [string, string] | undefined {
  const uci = puzzle.solution[state.index]
  return uci ? moveSquares(state.fen, uci) : undefined
}

export function playerColor(puzzle: Puzzle): 'white' | 'black' {
  return puzzle.fen.split(' ')[1] === 'b' ? 'black' : 'white'
}
