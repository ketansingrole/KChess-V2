import { INITIAL_FEN } from './position.ts'
import type { EngineLine } from '@kchess/contracts/types'
import { rules } from './engine.ts'

/**
 * The analysis board's move tree, like Lichess's: every position reached keeps the moves tried
 * from it, the first child being the main line and the rest its variations. A node is addressed
 * by its path, the UCI moves from the root joined with spaces (unique, since a move from a given
 * position is unique).
 *
 * Hosts hold the tree as a mutable (often reactive) object and keep references to its nodes, so
 * edits change the caller's tree in place and keep every node's identity. The decisions (what a
 * path means, whether a move is new, where a node goes, what a glyph is) are Rust's
 * (`crates/kchess-domain/src/trainer/analysis_tree.rs`); this module walks the tree and applies
 * what Rust returns.
 */
export interface TreeNode {
  /** Empty at the root. */
  uci: string
  san: string
  fen: string
  /** Half-moves since the game's start: 0 is White's first move still to play. */
  ply: number
  children: TreeNode[]
  /** PGN document metadata is stored on the root; annotations belong to each move. */
  headers?: Record<string, string>
  comments?: string[]
  startingComments?: string[]
  nags?: number[]
}

/** The move annotations (PGN NAGs 1–6) a study can mark a move with. */
export const MOVE_GLYPHS = [
  { nag: 1, glyph: '!', label: 'Good move' },
  { nag: 2, glyph: '?', label: 'Mistake' },
  { nag: 3, glyph: '!!', label: 'Brilliant move' },
  { nag: 4, glyph: '??', label: 'Blunder' },
  { nag: 5, glyph: '!?', label: 'Interesting move' },
  { nag: 6, glyph: '?!', label: 'Dubious move' },
] as const

/** The move's annotation glyph, if it has one. */
export function moveGlyph(node: Pick<TreeNode, 'nags'>): (typeof MOVE_GLYPHS)[number] | undefined {
  return rules<(typeof MOVE_GLYPHS)[number] | null>('moveGlyph', node.nags ?? null) ?? undefined
}

/** Mark a move with one annotation glyph (or none), keeping its other NAGs (evaluation symbols…). */
export function setMoveGlyph(node: TreeNode, nag: number | undefined): void {
  node.nags = rules<number[] | null>('setMoveGlyph', node.nags ?? null, nag ?? null) ?? undefined
}

export const pathOf = (moves: readonly string[]): string => rules<string>('pathOf', moves)
export const movesOf = (path: string): string[] => rules<string[]>('movesOf', path)

export function newTree(fen = INITIAL_FEN): TreeNode {
  // An unusable FEN starts from the usual position.
  const start = rules<{ fen: string; ply: number }>('startNode', fen)
  return { uci: '', san: '', fen: start.fen, ply: start.ply, children: [] }
}

/** The nodes from the root to `path`, as far as the path exists. */
export function nodesAlong(root: TreeNode, path: string): TreeNode[] {
  const nodes = [root]
  for (const uci of movesOf(path)) {
    const next = nodes.at(-1)!.children.find((child) => child.uci === uci)
    if (!next) break
    nodes.push(next)
  }
  return nodes
}

/** The index of each node along `nodes` among its parent's children (the first child is 0). */
const indicesAlong = (nodes: readonly TreeNode[]): number[] =>
  nodes.slice(1).map((node, index) => nodes[index]!.children.indexOf(node))

export const nodeAt = (root: TreeNode, path: string): TreeNode => nodesAlong(root, path).at(-1)!

/** Play `uci` after `path`, re-using the node when that move was already tried. */
export function addMove(root: TreeNode, path: string, uci: string): string | undefined {
  const parent = nodeAt(root, path)
  const plan = rules<{
    childPath: string
    child: { uci: string; san: string; fen: string; ply: number } | null
  } | null>('addMovePlan', {
    path,
    uci,
    fen: parent.fen,
    ply: parent.ply,
    childUcis: parent.children.map((child) => child.uci),
  })
  if (!plan) return undefined
  if (plan.child) parent.children.push({ ...plan.child, children: [] })
  return plan.childPath
}

/** Follow the first child from `path` to the end of that line. */
export function lineEnd(root: TreeNode, path: string): string {
  const chain: string[] = []
  let node = nodeAt(root, path)
  while (node.children[0]) {
    node = node.children[0]
    chain.push(node.uci)
  }
  return rules<string>('lineEnd', path, chain)
}

/** True when every move on `path` is a first child. */
export function onMainline(root: TreeNode, path: string): boolean {
  return rules<boolean>('onMainline', indicesAlong(nodesAlong(root, path)))
}

/** Remove the move at `path` and everything after it; returns the parent's path. */
export function deleteAt(root: TreeNode, path: string): string {
  const plan = rules<{ parentPath: string; uci: string | null }>('deleteAtPlan', path)
  if (plan.uci !== null) {
    const parent = nodeAt(root, plan.parentPath)
    // In place: the children array keeps its identity, as the nodes do.
    let kept = 0
    for (const child of parent.children) if (child.uci !== plan.uci) parent.children[kept++] = child
    parent.children.length = kept
  }
  return plan.parentPath
}

/** Make the line through `path` the main line at every branch on the way. */
export function promoteToMainline(root: TreeNode, path: string): void {
  const nodes = nodesAlong(root, path)
  for (const [depth, from] of rules<[number, number][]>('promotePlan', indicesAlong(nodes))) {
    const children = nodes[depth]!.children
    const [child] = children.splice(from, 1)
    children.unshift(child!)
  }
}

/** “1.”, “1…”: the number to print before a move at this ply (the ply *before* it is played). */
export function moveNumber(ply: number, always: boolean): string {
  return rules<string>('moveNumber', ply, always)
}

/* ── PGN ─────────────────────────────────────────────────────────────── */

export function treeToPgn(
  root: TreeNode,
  headers: Record<string, string> = root.headers ?? {},
): string {
  return rules<string>('treeToPgn', root, headers)
}

/** One bounded PGN document, or undefined when it cannot be imported without loss. */
export function treeFromPgn(text: string): TreeNode | undefined {
  return rules<TreeNode | null>('treeFromPgn', text) ?? undefined
}

/* ── Engine output ───────────────────────────────────────────────────── */

/** “+0.34”, “−1.20”, “#3”, “#−2”, as Lichess prints them. */
export function formatEval(line: Pick<EngineLine, 'cp' | 'mate'> | undefined): string {
  return rules<string>('formatEval', line ?? null)
}

/**
 * White's winning chances in −1…1 (Lichess's curve), for the evaluation bar: an even game is the
 * middle and a won one fills the bar, without ±10 pawns and ±1 pawn looking alike or apart.
 */
export function winningChances(line: Pick<EngineLine, 'cp' | 'mate'> | undefined): number {
  return rules<number>('winningChances', line ?? null)
}

/** A principal variation in SAN with move numbers, stopping at the first illegal move. */
const PV_SAN_CACHE_LIMIT = 200
const pvSanCache = new Map<string, { san: string; uci: string; label: string; fen: string }[]>()
/** Clear the `pvSan` memo (tests only). */
export function clearPvSanCache(): void {
  pvSanCache.clear()
}
export function pvSan(
  fen: string,
  pv: readonly string[],
  max = 12,
): { san: string; uci: string; label: string; fen: string }[] {
  const normalized = pv.slice(0, max)
  const key = `${fen}|${normalized.join(' ')}`
  const hit = pvSanCache.get(key)
  if (hit) {
    // Refresh LRU order so live engine updates keep the current lines hot.
    pvSanCache.delete(key)
    pvSanCache.set(key, hit)
    return hit
  }
  const moves = rules<{ san: string; uci: string; label: string; fen: string }[]>(
    'pvSan',
    fen,
    normalized,
    normalized.length,
  )
  pvSanCache.set(key, moves)
  if (pvSanCache.size > PV_SAN_CACHE_LIMIT) pvSanCache.delete(pvSanCache.keys().next().value!)
  return moves
}
