import { defineStore } from 'pinia'
import { watch } from 'vue'
import type { Judgment, Puzzle, PuzzleWrongMove, StoredReview } from '@kchess/contracts/types'
import { setupPgn } from '@kchess/rules/chess'
import {
  movesOf,
  nodesAlong,
  pathOf,
  setMoveGlyph,
  type TreeNode,
} from '@kchess/rules/analysisTree'
import { playerColor } from '@kchess/rules/puzzle'
import { analyseReview, reviewKey } from '@kchess/rules/review'
import { useAnalysisStore } from './analysis'
import { useReviewStore } from './review'

/** The NAG of each review label: ?! dubious, ? mistake, ?? blunder. */
const NAGS: Record<Judgment, number> = { inaccuracy: 6, mistake: 2, blunder: 4 }
/** The note on a move the player tried instead of the solution's. */
export const YOUR_MOVE = 'Your move'

/** The node at `path`, when every move of it is still on the board (variations can be deleted). */
export function variationNode(root: TreeNode, path: string): TreeNode | undefined {
  const nodes = nodesAlong(root, path)
  return nodes.length === movesOf(path).length + 1 ? nodes.at(-1) : undefined
}

/**
 * A puzzle studied on the analysis board: its solution is the line, each move the player tried
 * instead is a variation beside it, labelled once the engine has judged it, and the solution is
 * reviewed like a game. Kept here, not in the page that opened it, because the labels arrive after
 * that page is gone.
 */
export const usePuzzleAnalysis = defineStore('puzzleAnalysis', () => {
  const analysis = useAnalysisStore()
  const reviews = useReviewStore()

  /** The board still holds this puzzle (it starts from the puzzle's position). */
  const holds = (puzzle: Puzzle): boolean => analysis.root.fen === puzzle.fen
  /** The board's game as this store last left it: unchanged since, it is replaced without asking. */
  let left = ''
  const untouched = (): boolean => left !== '' && analysis.pgn() === left

  /** Judge `uci` from the position before it, and label its variation when the review is done. */
  async function label(puzzle: Puzzle, path: string, fen: string, uci: string): Promise<void> {
    const apply = (review: StoredReview | null | undefined): boolean => {
      if (!review?.complete) return false
      const judgment = analyseReview(review).moves[0]?.judgment
      const node = holds(puzzle) ? variationNode(analysis.root, path) : undefined
      if (judgment && node) {
        const unchanged = untouched()
        setMoveGlyph(node, NAGS[judgment])
        if (unchanged) left = analysis.pgn()
      }
      return true
    }
    let stored: StoredReview | null
    try {
      stored = await reviews.request({ fen, moves: [uci] })
    } catch (cause) {
      // Without an engine the move stays marked as yours, unlabelled.
      console.warn('[puzzle-analysis] Judging a tried move failed:', cause)
      return
    }
    if (apply(stored)) return
    const key = stored?.key ?? reviewKey(fen, [uci])
    const stop = watch(
      () => reviews.reviews.get(key),
      (review) => {
        if (apply(review)) stop()
      },
    )
  }

  /** Put the puzzle on the board (the page asks first when that replaces unsaved analysis). */
  function open(puzzle: Puzzle, wrongMoves: readonly PuzzleWrongMove[] = []): boolean {
    const pgn = setupPgn({ variant: 'standard', fen: puzzle.fen }, puzzle.solution)
    if (!analysis.loadPgn(pgn)) return false
    const player = playerColor(puzzle)
    const tried: { path: string; fen: string; uci: string }[] = []
    for (const { index, uci } of wrongMoves) {
      const before = pathOf(puzzle.solution.slice(0, index))
      analysis.goTo(before)
      // Only a move that still is a legal alternative here becomes a variation.
      if (analysis.path !== before || uci === puzzle.solution[index]) continue
      const fen = analysis.node.fen
      if (!analysis.play(uci)) continue
      analysis.node.comments = [YOUR_MOVE]
      tried.push({ path: analysis.path, fen, uci })
    }
    analysis.goTo('')
    analysis.orientation = player
    analysis.origin = { player, puzzle: { ...puzzle, wrongMoves: [...wrongMoves] } }
    analysis.startReview()
    left = analysis.pgn()
    for (const move of tried) void label(puzzle, move.path, move.fen, move.uci)
    return true
  }

  /** The board's pieces, side to move, castling and en passant: a position without its clocks. */
  const placement = (fen: string): string => fen.split(' ').slice(0, 4).join(' ')

  /**
   * The game the puzzle comes from (`pgn`, as Lichess exports it) on the board, at the puzzle's
   * position, reviewed as a game. The puzzle stays with it, so the board can go back to it.
   */
  function showSource(puzzle: Puzzle & { wrongMoves: PuzzleWrongMove[] }, pgn: string): boolean {
    if (!analysis.loadPgn(pgn)) return false
    let ply = 0
    let node: TreeNode | undefined = analysis.root
    for (let at = 0; node; node = node.children[0], at++) {
      if (placement(node.fen) === placement(puzzle.fen)) {
        ply = at
        break
      }
    }
    analysis.orientation = playerColor(puzzle)
    analysis.origin = { gameId: puzzle.gameId, puzzle }
    analysis.startReview()
    analysis.goToPly(ply)
    left = analysis.pgn()
    return true
  }

  return { open, holds, untouched, showSource }
})
