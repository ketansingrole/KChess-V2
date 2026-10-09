import { treeFromPgn, type TreeNode } from './analysisTree'

/** What a study's card shows: its final position and how much is in it. */
export interface StudySummary {
  fen: string
  /** Half-moves in the main line. */
  plies: number
  comments: number
  variations: number
  players: string
  event: string
}

const cache = new Map<string, StudySummary | null>()

/** Summarise a study's PGN (remembered, since the library page shows every study at once). */
export function summarizeStudy(pgn: string): StudySummary | null {
  const known = cache.get(pgn)
  if (known !== undefined) return known
  const root = treeFromPgn(pgn)
  const summary = root ? summarize(root) : null
  if (cache.size > 200) cache.clear()
  cache.set(pgn, summary)
  return summary
}

function summarize(root: TreeNode): StudySummary {
  let comments = root.comments?.length ? 1 : 0
  let variations = 0
  const stack = [root]
  while (stack.length) {
    const node = stack.pop()!
    variations += Math.max(0, node.children.length - 1)
    for (const child of node.children) {
      if (child.comments?.length) comments++
      stack.push(child)
    }
  }
  let end = root
  while (end.children[0]) end = end.children[0]
  const known = (value?: string): string => (value && value !== '?' ? value : '')
  const white = known(root.headers?.White)
  const black = known(root.headers?.Black)
  return {
    fen: end.fen,
    plies: end.ply - root.ply,
    comments,
    variations,
    players: white || black ? `${white || '?'} – ${black || '?'}` : '',
    event: known(root.headers?.Event),
  }
}
