import { Position, repetitionKey } from './position.ts'

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
 */
export const ENDGAME_DRILLS: readonly EndgameDrill[] = [
  {
    id: 'kq-vs-k',
    title: 'King and queen vs king',
    group: 'Basic mates',
    fen: '4k3/8/8/8/8/8/8/3QK3 w - - 0 1',
    player: 'white',
    goal: 'win',
    level: 'Easy',
    idea: 'Box the king in with the queen, bring your king up, then mate. Watch out for stalemate.',
  },
  {
    id: 'kr-vs-k',
    title: 'King and rook vs king',
    group: 'Basic mates',
    fen: '4k3/8/8/8/8/8/8/R3K3 w - - 0 1',
    player: 'white',
    goal: 'win',
    level: 'Medium',
    idea: 'Cut the king off with the rook and push it to the edge with your king’s help.',
  },
  {
    id: 'kbb-vs-k',
    title: 'Two bishops vs king',
    group: 'Basic mates',
    fen: '4k3/8/8/8/8/8/8/2B1KB2 w - - 0 1',
    player: 'white',
    goal: 'win',
    level: 'Medium',
    idea: 'The bishops cover neighbouring diagonals; walk them and your king toward a corner.',
  },
  {
    id: 'kbn-vs-k',
    title: 'Bishop and knight vs king',
    group: 'Basic mates',
    fen: '4k3/8/8/8/8/8/8/2B1K1N1 w - - 0 1',
    player: 'white',
    goal: 'win',
    level: 'Hard',
    idea: 'Drive the king to a corner of the bishop’s colour. It has to happen inside 50 moves.',
  },
  {
    id: 'kp-key-squares',
    title: 'King in front of the pawn',
    group: 'Pawn endings',
    fen: '4k3/8/4K3/4P3/8/8/8/8 w - - 0 1',
    player: 'white',
    goal: 'win',
    level: 'Easy',
    idea: 'Your king is on the key square in front of the pawn: lead the pawn home with it.',
  },
  {
    id: 'kp-opposition',
    title: 'Win the opposition',
    group: 'Pawn endings',
    fen: '8/8/8/3k4/8/3K4/3P4/8 b - - 0 1',
    player: 'white',
    goal: 'win',
    level: 'Medium',
    idea: 'Black has to give way. Take the opposition at the right moment and the pawn queens.',
  },
  {
    id: 'kp-hold',
    title: 'Hold the draw against a pawn',
    group: 'Pawn endings',
    fen: '8/8/3k4/3P4/3K4/8/8/8 b - - 0 1',
    player: 'black',
    goal: 'draw',
    level: 'Medium',
    idea: 'You defend: keep the opposition and never let the white king in front of its pawn.',
  },
  {
    id: 'lucena',
    title: 'Lucena position',
    group: 'Rook endings',
    fen: '1K1k4/1P6/8/8/8/8/r7/2R5 w - - 0 1',
    player: 'white',
    goal: 'win',
    level: 'Hard',
    idea: 'Build a bridge: get the rook to the fourth rank and shelter the king from the checks.',
  },
  {
    id: 'philidor',
    title: 'Philidor position',
    group: 'Rook endings',
    fen: '4k3/8/r7/3KP3/8/8/8/7R b - - 0 1',
    player: 'black',
    goal: 'draw',
    level: 'Hard',
    idea: 'Keep your rook on the sixth rank until the pawn advances, then check from behind.',
  },
]

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
  const start = Position.fromFen(fen)
  if (!start) throw new Error('Invalid drill position.')
  const line = start.line(moves)
  const pos = start.after(line)
  const seen = new Map<string, number>([[repetitionKey(start.fen), 1]])
  let repeated = false
  for (const move of line) {
    const key = repetitionKey(move.fen)
    const count = (seen.get(key) ?? 0) + 1
    seen.set(key, count)
    if (count >= 3) repeated = true
  }
  const base = { turn: pos.turn, fen: pos.fen }
  if (pos.isCheckmate()) {
    const won = pos.turn !== player
    return {
      ...base,
      over: true,
      success: won,
      title: won ? 'Checkmate — well done!' : 'You were checkmated',
      detail: won ? undefined : 'Try the position again.',
    }
  }
  const drawn = pos.isStalemate()
    ? 'Stalemate'
    : pos.isInsufficientMaterial()
      ? 'Insufficient material'
      : repeated
        ? 'Threefold repetition'
        : pos.halfmoves >= 100
          ? 'Fifty-move rule'
          : undefined
  if (drawn) {
    const success = goal === 'draw'
    return {
      ...base,
      over: true,
      success,
      title: success ? `Draw held — ${drawn.toLowerCase()}` : `Draw — ${drawn.toLowerCase()}`,
      detail: success
        ? 'That is the result to aim for here.'
        : 'This position is winning; try again.',
    }
  }
  return {
    ...base,
    over: false,
    title: pos.isCheck() ? 'Check!' : `${pos.turn === 'white' ? 'White' : 'Black'} to play`,
  }
}

/** SAN of a UCI move list played from `fen`. */
export function sanFrom(fen: string, moves: readonly string[]): string[] {
  return (
    Position.fromFen(fen)
      ?.line(moves)
      .map((move) => move.san) ?? []
  )
}
