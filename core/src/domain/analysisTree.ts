import { INITIAL_FEN } from 'chessops/fen'
import type { EngineLine } from '../contracts/types'
import { rules } from './engine.ts'

/**
 * The analysis board's move tree, like Lichess's: every position reached keeps the moves tried
 * from it, the first child being the main line and the rest its variations. A node is addressed
 * by its path, the UCI moves from the root joined with spaces (unique, since a move from a given
 * position is unique).
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
  return MOVE_GLYPHS.find((entry) => node.nags?.includes(entry.nag))
}
/** Mark a move with one annotation glyph (or none), keeping its other NAGs (evaluation symbols…). */
export function setMoveGlyph(node: TreeNode, nag: number | undefined): void {
  const others = (node.nags ?? []).filter((n) => n < 1 || n > 6)
  const next = nag ? [nag, ...others] : others
  node.nags = next.length ? next : undefined
}

export const pathOf = (moves: readonly string[]): string => moves.join(' ')
export const movesOf = (path: string): string[] => (path ? path.split(' ') : [])

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

export const nodeAt = (root: TreeNode, path: string): TreeNode => nodesAlong(root, path).at(-1)!

/** Play `uci` after `path`, re-using the node when that move was already tried. */
export function addMove(root: TreeNode, path: string, uci: string): string | undefined {
  const parent = nodeAt(root, path)
  // Castling comes back as king-takes-rook, so a move has one spelling in the tree.
  const played = rules<{ uci: string; san: string; fen: string } | null>(
    'playMove',
    parent.fen,
    uci,
  )
  if (!played) return undefined
  const childPath = pathOf([...movesOf(path), played.uci])
  if (parent.children.some((child) => child.uci === played.uci)) return childPath
  parent.children.push({ ...played, ply: parent.ply + 1, children: [] })
  return childPath
}

/** Follow the first child from `path` to the end of that line. */
export function lineEnd(root: TreeNode, path: string): string {
  const moves = movesOf(path)
  let node = nodeAt(root, path)
  while (node.children[0]) {
    node = node.children[0]
    moves.push(node.uci)
  }
  return pathOf(moves)
}

/** True when every move on `path` is a first child. */
export function onMainline(root: TreeNode, path: string): boolean {
  const nodes = nodesAlong(root, path)
  return nodes.every((node, index) => index === 0 || nodes[index - 1]!.children[0] === node)
}

/** Remove the move at `path` and everything after it; returns the parent's path. */
export function deleteAt(root: TreeNode, path: string): string {
  const moves = movesOf(path)
  const uci = moves.pop()
  if (!uci) return ''
  const parentPath = pathOf(moves)
  const parent = nodeAt(root, parentPath)
  parent.children = parent.children.filter((child) => child.uci !== uci)
  return parentPath
}

/** Make the line through `path` the main line at every branch on the way. */
export function promoteToMainline(root: TreeNode, path: string): void {
  const nodes = nodesAlong(root, path)
  for (let i = 1; i < nodes.length; i++) {
    const parent = nodes[i - 1]!
    const node = nodes[i]!
    parent.children = [node, ...parent.children.filter((child) => child !== node)]
  }
}

/** “1.”, “1…”: the number to print before a move at this ply (the ply *before* it is played). */
export function moveNumber(ply: number, always: boolean): string {
  const number = Math.floor(ply / 2) + 1
  if (ply % 2 === 0) return `${number}.`
  return always ? `${number}…` : ''
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
  if (!line) return '…'
  if (line.mate !== undefined) return `#${line.mate < 0 ? '−' : ''}${Math.abs(line.mate)}`
  const pawns = (line.cp ?? 0) / 100
  return `${pawns > 0 ? '+' : pawns < 0 ? '−' : ''}${Math.abs(pawns).toFixed(1)}`
}

/**
 * White's winning chances in −1…1 (Lichess's curve), for the evaluation bar: an even game is the
 * middle and a won one fills the bar, without ±10 pawns and ±1 pawn looking alike or apart.
 */
export function winningChances(line: Pick<EngineLine, 'cp' | 'mate'> | undefined): number {
  if (!line) return 0
  if (line.mate !== undefined) return line.mate > 0 ? 1 : line.mate < 0 ? -1 : 0
  const cp = Math.max(-1000, Math.min(1000, line.cp ?? 0))
  return 2 / (1 + Math.exp(-0.00368208 * cp)) - 1
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
