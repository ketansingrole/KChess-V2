/** Turning puzzles from Lichess (API or database) into one shape, and the move rules of solving them. */
import { Position, uciSquares } from './position.ts'
import type { Puzzle } from '../contracts/types.ts'

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

/**
 * From a Lichess API puzzle. Current responses carry the starting `fen` and `lastMove`; older ones
 * only the game's moves, of which the first `initialPly + 1` lead to the puzzle.
 */
export function puzzleFromApi(raw: ApiPuzzle): Puzzle | undefined {
  const { puzzle, game } = raw
  let fen = puzzle.fen
  let lastMove = puzzle.lastMove
  if (!fen || !lastMove) {
    const start = Position.initial()
    const sans = (game.pgn ?? '').split(/\s+/).filter((token) => token && !/^\d+\.+$/.test(token))
    const wanted = sans.slice(0, puzzle.initialPly + 1)
    const line = start.line(wanted, { san: true })
    if (line.length < wanted.length) return undefined
    lastMove = line.at(-1)?.uci ?? lastMove
    fen = line.at(-1)?.fen ?? start.fen
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
  const opening = first ? Position.fromFen(row.fen)?.play(first) : undefined
  if (!opening || !solution.length) return undefined
  return {
    id: row.id,
    fen: opening.position.fen,
    lastMove: first,
    solution,
    rating: row.rating,
    themes: row.themes.split(' ').filter(Boolean),
    plays: row.plays,
  }
}

/** Squares to highlight for a UCI move; castling shows the king's two-square step, not "king takes rook". */
export function moveSquares(fen: string, uci: string): [string, string] | undefined {
  const squares = uciSquares(uci)
  if (!squares) return undefined
  const [from, to] = squares
  const side = castlingSide(Position.fromFen(fen), from, to)
  if (!side) return [from, to]
  return [from, `${side === 'h' ? 'g' : 'c'}${from[1]}`]
}

/** chessops `castlingSide`: a king moving two files, or onto its own piece, castles. */
function castlingSide(pos: Position | undefined, from: string, to: string): 'a' | 'h' | undefined {
  if (!pos) return undefined
  const index = (square: string): number => square.charCodeAt(0) - 97 + 8 * (Number(square[1]) - 1)
  const delta = index(to) - index(from)
  if (Math.abs(delta) !== 2 && pos.pieceAt(to)?.color !== pos.turn) return undefined
  if (pos.pieceAt(from)?.role !== 'king') return undefined
  return delta > 0 ? 'h' : 'a'
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

/** Both are the same legal move (castling may be written either way). */
function sameMove(pos: Position, a: string, b: string): boolean {
  const first = pos.play(a)
  return first !== undefined && first.uci === pos.play(b)?.uci
}

/** Play `uci` from `state`; returns the new state, or undefined when the move is illegal. */
function applyMove(state: PuzzleState, uci: string): PuzzleState | undefined {
  const played = Position.fromFen(state.fen)?.play(uci)
  if (!played) return undefined
  const squares = moveSquares(state.fen, uci)
  return {
    ...state,
    fen: played.position.fen,
    lastMove: squares,
    sans: [...state.sans, played.san],
  }
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
  const pos = Position.fromFen(state.fen)
  const expected = puzzle.solution[state.index]
  if (state.status !== 'playing' || !pos || !expected) return { state, correct: false }
  const played = pos.play(uci)
  if (!played) return { state, correct: false }
  const mates = played.position.isCheckmate()
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
