import { rules } from './engine.ts'
import training from './data/training.json' with { type: 'json' }

export type DrillGoal = 'win' | 'draw'

export interface EndgameDrill {
  id: string
  title: string
  group: 'Basic mates' | 'Pawn endings' | 'Rook endings'
  fen: string
  /** The side you play. */
  player: 'white' | 'black'
  goal: DrillGoal
  level: 'Easy' | 'Medium' | 'Hard'
  /** What to aim for, in a sentence. */
  idea: string
}

/**
 * Every position was checked with Stockfish at depth 24: the "win" ones are won for the side you
 * play and the "draw" ones are drawn with best play, so the engine's replies are the real test.
 * The drills are shared data (`data/training.json`), read by the Rust rules as well.
 */
export const ENDGAME_DRILLS = training.endgameDrills as readonly EndgameDrill[]

export interface EndgameStatus {
  over: boolean
  /** Only once `over`. */
  success?: boolean
  title: string
  detail?: string
}

/** Where the drill stands after `moves` (UCI, from `fen`), judged for `player` and the drill's goal. */
export function evaluateEndgame(
  fen: string,
  moves: readonly string[],
  player: 'white' | 'black',
  goal: DrillGoal,
): EndgameStatus & { turn: 'white' | 'black'; fen: string } {
  return rules('evaluateEndgame', fen, [...moves], player, goal)
}

/** SAN of a UCI move list played from `fen`. */
export function sanFrom(fen: string, moves: readonly string[]): string[] {
  return rules<string[]>('sanFrom', fen, [...moves])
}
