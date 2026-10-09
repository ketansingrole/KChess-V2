import { Chess, normalizeMove, type Position } from 'chessops/chess'
import { INITIAL_FEN, makeFen } from 'chessops/fen'
import {
  ChildNode,
  defaultGame,
  makePgn,
  parsePgn,
  setStartingPosition,
  startingPosition,
  type Node,
  type PgnNodeData,
} from 'chessops/pgn'
import { makeSanAndPlay, parseSan } from 'chessops/san'
import { makeUci, parseUci } from 'chessops/util'
import type { EngineLine } from '../contracts/types'
import { positionFromFen } from './chess'

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

function plyOf(pos: Position): number {
  return (pos.fullmoves - 1) * 2 + (pos.turn === 'white' ? 0 : 1)
}

export function newTree(fen = INITIAL_FEN): TreeNode {
  const pos = positionFromFen(fen) ?? Chess.default()
  return { uci: '', san: '', fen: makeFen(pos.toSetup()), ply: plyOf(pos), children: [] }
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
  const pos = positionFromFen(parent.fen)
  const parsed = parseUci(uci)
  if (!pos || !parsed) return undefined
  const move = normalizeMove(pos, parsed)
  if (!pos.isLegal(move)) return undefined
  uci = makeUci(move)
  const existing = parent.children.find((child) => child.uci === uci)
  const childPath = pathOf([...movesOf(path), uci])
  if (existing) return childPath
  const san = makeSanAndPlay(pos, move)
  parent.children.push({ uci, san, fen: makeFen(pos.toSetup()), ply: parent.ply + 1, children: [] })
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
  const game = defaultGame<PgnNodeData>()
  for (const [key, value] of Object.entries(headers)) game.headers.set(key, value)
  game.comments = root.comments
  const pos = positionFromFen(root.fen)
  if (pos && root.fen !== INITIAL_FEN) setStartingPosition(game.headers, pos)
  const copy = (from: TreeNode, to: Node<PgnNodeData>): void => {
    for (const child of from.children) {
      const node = new ChildNode<PgnNodeData>({
        san: child.san,
        comments: child.comments,
        startingComments: child.startingComments,
        nags: child.nags,
      })
      to.children.push(node)
      copy(child, node)
    }
  }
  copy(root, game.moves)
  return makePgn(game)
}

/** One bounded PGN document, or undefined when it cannot be imported without loss. */
export function treeFromPgn(text: string): TreeNode | undefined {
  if (text.length > 2_000_000) return undefined
  const games = parsePgn(text)
  if (games.length !== 1) return undefined
  const game = games[0]!
  const start = startingPosition(game.headers)
  if (start.isErr) return undefined
  const root = newTree(makeFen(start.value.toSetup()))
  root.headers = Object.fromEntries(game.headers)
  root.comments = game.comments
  let invalid = false
  let count = 0
  const walk = (from: Node<PgnNodeData>, to: TreeNode, pos: Position, depth = 0): void => {
    if (depth > 1024) {
      invalid = true
      return
    }
    for (const child of from.children) {
      if (++count > 10_000) {
        invalid = true
        return
      }
      const position = pos.clone()
      const move = parseSan(position, child.data.san)
      if (!move) {
        invalid = true
        continue
      }
      const uci = makeUci(move)
      const san = makeSanAndPlay(position, move)
      const node: TreeNode = {
        uci,
        san,
        fen: makeFen(position.toSetup()),
        ply: to.ply + 1,
        children: [],
        comments: child.data.comments,
        startingComments: child.data.startingComments,
        nags: child.data.nags,
      }
      to.children.push(node)
      walk(child, node, position, depth + 1)
    }
  }
  walk(game.moves, root, start.value)
  return invalid ? undefined : root
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
  const pos = positionFromFen(fen)
  if (!pos) return []
  const moves: { san: string; uci: string; label: string; fen: string }[] = []
  let ply = plyOf(pos)
  for (const uci of normalized) {
    const parsed = parseUci(uci)
    if (!parsed) break
    const move = normalizeMove(pos, parsed)
    if (!pos.isLegal(move)) break
    const number = moveNumber(ply, moves.length === 0)
    const san = makeSanAndPlay(pos, move)
    moves.push({ san, uci, label: number ? `${number} ${san}` : san, fen: makeFen(pos.toSetup()) })
    ply++
  }
  pvSanCache.set(key, moves)
  if (pvSanCache.size > PV_SAN_CACHE_LIMIT) pvSanCache.delete(pvSanCache.keys().next().value!)
  return moves
}
