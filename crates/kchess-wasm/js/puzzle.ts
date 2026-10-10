/** Turning puzzles from Lichess (API or database) into one shape, and the move rules of solving them. */
import { rules } from './engine.ts'
import type { Puzzle } from '@kchess/contracts/types'

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
  return rules<Puzzle | null>('puzzleFromApi', raw) ?? undefined
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
  return rules<Puzzle | null>('puzzleFromDb', row) ?? undefined
}

/** Squares to highlight for a UCI move; castling shows the king's two-square step, not "king takes rook". */
export function moveSquares(fen: string, uci: string): [string, string] | undefined {
  return rules<[string, string] | null>('moveSquares', fen, uci) ?? undefined
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
  return rules<PuzzleState>('startPuzzle', puzzle)
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
  return rules<PlayerMove>('playerMove', puzzle, state, uci)
}

/** Play the opponent's scripted reply. */
export function opponentReply(state: PuzzleState, uci: string): PuzzleState {
  return rules<PuzzleState>('opponentReply', state, uci)
}

/** Squares of the next solution move, for a hint (`from`) or the solution arrow. */
export function nextSolutionSquares(
  puzzle: Puzzle,
  state: PuzzleState,
): [string, string] | undefined {
  return rules<[string, string] | null>('nextSolutionSquares', puzzle, state) ?? undefined
}

export function playerColor(puzzle: Puzzle): 'white' | 'black' {
  return rules<'white' | 'black'>('playerColor', puzzle)
}
