import type { LookupMove } from '@kchess/contracts/types'
import { rules } from './engine.ts'
import type { GameSetup } from './variant.ts'

export interface OpeningName {
  eco: string
  name: string
}

/**
 * One entry of the openings tree (Lichess's CC0 named lines, `tooling/make-openings.mjs`): a family
 * holds the families named after it and its variations, which hold their subvariations. An entry
 * no line is named for exactly only groups its children, and has no `eco` or `moves`.
 */
export interface OpeningNode {
  /** The full name, as `openingAt` gives it. */
  name: string
  /** The part of the name below the parent entry (“Accepted” under “King's Gambit”). */
  label: string
  eco?: string
  /** The shortest line with this name, as UCI moves from the starting position. */
  moves?: string[]
  children: OpeningNode[]
}

/**
 * The name of the opening reached after `ply` moves. As Lichess suggests, step back from the
 * position until a named one is found. Only games under standard rules have opening names.
 * The table lives in the Rust rules (`crates/kchess-domain/src/training/openings.rs`).
 */
export function openingAt(
  setup: GameSetup,
  moves: readonly string[],
  ply = moves.length,
): OpeningName | undefined {
  // A non-finite `ply` travels as its text, which the rules read as the number it stands for.
  const wire = Number.isFinite(ply) ? ply : String(ply)
  return rules<OpeningName | null>('openingAt', setup, [...moves], wire) ?? undefined
}

/**
 * The key Lichess names an opening's puzzles by (its family and first variation), the puzzle
 * angle for “Train with puzzles”: `Kings_Gambit_Accepted_Kieseritzky_Gambit`.
 */
export function openingKey(name: string): string {
  return rules<string>('openingKey', name)
}

let tree: OpeningNode[] | undefined
/** Every named opening by family, alphabetically at each level; built once. */
export function openingTree(): OpeningNode[] {
  tree ??= rules<OpeningNode[]>('openingTree')
  return tree
}

/**
 * The moves an opening drill continues with from a position the explorer knows: for the player,
 * the most played move; for the opponent, the popular replies.
 */
export function openingReplies(
  moves: readonly Pick<LookupMove, 'uci' | 'white' | 'draws' | 'black'>[],
  yours: boolean,
): string[] {
  return rules<string[]>(
    'openingReplies',
    moves.map(({ uci, white, draws, black }) => ({ uci, white, draws, black })),
    yours,
  )
}
