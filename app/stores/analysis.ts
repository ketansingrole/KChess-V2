import { defineStore } from 'pinia'
import { computed, effectScope, ref, watch, type EffectScope } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import type { Color } from '@lichess-org/chessground/types'
import type { AnalysisUpdate, Judgment } from '../../src/shared/types'
import { analyseReview, reviewKey, type GameAnalysis } from '../../src/shared/review'
import { useReviewStore } from './review'
import {
  addMove,
  deleteAt,
  lineEnd,
  movesOf,
  newTree,
  nodesAlong,
  pathOf,
  promoteToMainline,
  treeFromPgn,
  treeToPgn,
  type TreeNode,
} from '../utils/analysisTree'
import { playUci, positionFromFen } from '../utils/chess'
import { setupFromFen, START_SETUP, type EditorSetup } from '../utils/boardEditor'

/** Evaluations kept per position, so stepping back and forth shows them at once. */
const CACHE_SIZE = 600

/** Where the game on the board came from, for its review and the players' names. */
export interface GameOrigin {
  gameId?: string
  account?: string
  white?: string
  black?: string
}

/** What the review says about one main-line move. */
export interface ReviewMark {
  /** Index of the move in the game (0 is the first move). */
  index: number
  color: 'white' | 'black'
  judgment?: Judgment
  /** The engine's move instead of this one, and its line, from the position before it. */
  best?: { uci: string; san: string; pv: string[] }
}

/**
 * The analysis board and board editor. Both live here rather than in their pages, so a position
 * survives leaving the page, and the editor can hand its position to analysis (and back).
 */
export const useAnalysisStore = defineStore('analysis', () => {
  const root = ref<TreeNode>(newTree())
  const path = ref('')
  const orientation = ref<Color>('white')
  /** The board editor's position, kept while you visit other pages. */
  const editor = ref<EditorSetup>(structuredClone(START_SETUP))
  const editorOrientation = ref<Color>('white')

  const engineOn = useLocalStorage('kchess:analysis-engine', true)
  const engineLines = useLocalStorage('kchess:analysis-lines', 3)
  const infinite = useLocalStorage('kchess:analysis-infinite', false)
  const showArrows = useLocalStorage('kchess:analysis-arrows', true)

  const nodes = computed(() => nodesAlong(root.value, path.value))
  const node = computed(() => nodes.value.at(-1)!)
  const position = computed(() => positionFromFen(node.value.fen))

  /* ── Navigation and editing the tree ──────────────────────────────── */

  function load(fen?: string): void {
    root.value = newTree(fen)
    path.value = ''
    evaluations.clear()
    origin.value = null
  }
  /** Load a game; `ply` opens it at that many moves into the main line (default: the start). */
  function loadPgn(text: string, ply = 0): boolean {
    const tree = treeFromPgn(text)
    if (!tree) return false
    root.value = tree
    path.value = pathOf(movesOf(lineEnd(tree, '')).slice(0, Math.max(0, ply)))
    evaluations.clear()
    origin.value = null
    return true
  }
  function play(uci: string): boolean {
    const next = addMove(root.value, path.value, uci)
    if (next === undefined) return false
    path.value = next
    return true
  }
  function goTo(next: string): void {
    path.value = pathOf(
      nodesAlong(root.value, next)
        .map((n) => n.uci)
        .slice(1),
    )
  }
  function back(): void {
    goTo(pathOf(movesOf(path.value).slice(0, -1)))
  }
  function forward(): void {
    const child = node.value.children[0]
    if (child) goTo(pathOf([...movesOf(path.value), child.uci]))
  }
  function toStart(): void {
    path.value = ''
  }
  function toEnd(): void {
    path.value = lineEnd(root.value, path.value)
  }
  function remove(at = path.value): void {
    if (!at) return
    const parent = deleteAt(root.value, at)
    // Keep the board where it was unless the shown move was the one removed.
    if (path.value === at || path.value.startsWith(`${at} `)) path.value = parent
  }
  function promote(at = path.value): void {
    promoteToMainline(root.value, at)
  }
  function pgn(): string {
    return treeToPgn(root.value)
  }

  /* ── Engine ──────────────────────────────────────────────────────── */

  const evaluations = new Map<string, AnalysisUpdate>()
  const live = ref<AnalysisUpdate | null>(null)
  const engineError = ref('')
  const engineBusy = ref(false)
  let wanted = ''
  let latestId = 0
  let scope: EffectScope | undefined
  let off: (() => void) | undefined

  /** The deepest evaluation known for the position on the board. */
  const evaluation = computed<AnalysisUpdate | null>(() => {
    const fen = node.value.fen
    if (live.value?.fen === fen) return live.value
    return evaluations.get(fen) ?? null
  })
  const gameOver = computed(() => {
    const pos = position.value
    if (!pos) return ''
    if (pos.isCheckmate()) return 'Checkmate'
    if (pos.isStalemate()) return 'Stalemate'
    if (pos.isInsufficientMaterial()) return 'Insufficient material'
    return ''
  })

  function remember(update: AnalysisUpdate): void {
    const known = evaluations.get(update.fen)
    if (!update.lines.length) return
    if (known && known.depth > update.depth && known.lines.length >= update.lines.length) return
    evaluations.delete(update.fen)
    evaluations.set(update.fen, update)
    if (evaluations.size > CACHE_SIZE) evaluations.delete(evaluations.keys().next().value!)
  }

  function receive(update: AnalysisUpdate): void {
    // Updates for a position no longer on the board (or an older request for it) are stale.
    if (update.fen !== wanted || update.id < latestId) return
    latestId = update.id
    if (update.error) {
      engineError.value = update.error
      engineBusy.value = false
      return
    }
    live.value = update
    remember(update)
    if (update.done) engineBusy.value = false
  }

  async function analyse(): Promise<void> {
    const fen = node.value.fen
    engineError.value = ''
    if (!engineOn.value || gameOver.value || !position.value) {
      wanted = ''
      engineBusy.value = false
      void window.kchess.stopAnalysis().catch(() => {})
      return
    }
    const known = evaluations.get(fen)
    // A finished search with enough lines needs no new one, unless analysis is meant to go on.
    if (!infinite.value && known?.done && known.lines.length >= engineLines.value) {
      wanted = fen
      engineBusy.value = false
      void window.kchess.stopAnalysis().catch(() => {})
      return
    }
    wanted = fen
    live.value = null
    engineBusy.value = true
    try {
      const id = await window.kchess.startAnalysis({
        fen,
        lines: Math.max(1, Math.min(5, engineLines.value)),
        infinite: infinite.value,
      })
      if (wanted === fen) latestId = Math.max(latestId, id)
    } catch (cause) {
      if (wanted !== fen) return
      engineBusy.value = false
      engineError.value = cause instanceof Error ? cause.message : String(cause)
    }
  }

  /** Start following the board with the engine (the analysis page is open). */
  function attach(): void {
    if (scope) return
    off = window.kchess.onAnalysis(receive)
    scope = effectScope()
    scope.run(() => {
      watch(
        () => [node.value.fen, engineOn.value, engineLines.value, infinite.value] as const,
        () => void analyse(),
        { immediate: true },
      )
    })
  }
  function detach(): void {
    scope?.stop()
    scope = undefined
    off?.()
    off = undefined
    wanted = ''
    engineBusy.value = false
    void window.kchess.stopAnalysis().catch(() => {})
  }

  /* ── Game review ─────────────────────────────────────────────────── */

  const reviews = useReviewStore()
  const origin = ref<GameOrigin | null>(null)
  const mainline = computed(() => movesOf(lineEnd(root.value, '')))
  const mainlineKey = computed(() => reviewKey(root.value.fen, mainline.value))
  /** The review shown: of the main line, or of the game before moves were added to it. */
  const reviewShown = ref('')
  const review = computed(() => reviews.reviews.get(reviewShown.value) ?? null)
  // Moves along a shared start are the same positions, so an earlier review of the line still
  // labels them correctly; it is only replaced once a review of this exact line turns up.
  watch(
    mainlineKey,
    async (key) => {
      if (reviews.reviews.has(key)) {
        reviewShown.value = key
        return
      }
      const found = await reviews.load(root.value.fen, mainline.value)
      if (found && key === mainlineKey.value) reviewShown.value = found.key
    },
    { immediate: true },
  )
  // A review of this line that finishes (or arrives from Lichess) while it is on the board.
  watch(
    () => reviews.reviews.get(mainlineKey.value),
    (found) => {
      if (found) reviewShown.value = found.key
    },
  )
  /** How many main-line moves the review covers (it may be of a shorter or diverging line). */
  const reviewedPlies = computed(() => {
    const moves = review.value?.moves ?? []
    let i = 0
    while (i < moves.length && moves[i] === mainline.value[i]) i++
    return i
  })
  /** The summary applies: the review is of this main line, or of the start of it. */
  const reviewWhole = computed(
    () => !!review.value && reviewedPlies.value === review.value.moves.length,
  )
  const gameAnalysis = computed<GameAnalysis | null>(() =>
    review.value && review.value.fen === root.value.fen ? analyseReview(review.value) : null,
  )
  /** The review's verdict on each main-line move, by path. */
  const reviewMarks = computed(() => {
    const marks = new Map<string, ReviewMark>()
    const analysed = gameAnalysis.value
    const stored = review.value
    if (!analysed || !stored) return marks
    let parent = root.value
    for (let i = 0; i < reviewedPlies.value; i++) {
      const child = parent.children[0]
      const move = analysed.moves[i]
      if (!child || !move) break
      const mark: ReviewMark = { index: i, color: move.color, judgment: move.judgment }
      const before = stored.evals[i]
      if (move.judgment && before?.best && before.best !== child.uci) {
        const pos = positionFromFen(parent.fen)
        const san = pos && playUci(pos, before.best)
        if (san) mark.best = { uci: before.best, san, pv: before.pv ?? [before.best] }
      }
      marks.set(pathOf(mainline.value.slice(0, i + 1)), mark)
      parent = child
    }
    return marks
  })
  /** The review queue is working on this game. */
  const reviewProgress = computed(() => {
    const current = reviews.status.current
    return current && current.key === mainlineKey.value ? current : null
  })
  async function requestReview(): Promise<void> {
    if (!mainline.value.length) return
    const stored = await reviews.request({
      fen: root.value.fen,
      moves: mainline.value,
      gameId: origin.value?.gameId,
      account: origin.value?.account,
    })
    reviewShown.value = stored?.key ?? mainlineKey.value
  }
  function cancelReview(): void {
    void reviews.cancel(mainlineKey.value)
  }

  /* ── Handing positions between the editor and analysis ────────────── */

  function analyseSetup(fen: string): void {
    load(fen)
    orientation.value = editorOrientation.value
  }
  function editPosition(fen = node.value.fen): void {
    editor.value = setupFromFen(fen) ?? structuredClone(START_SETUP)
    editorOrientation.value = orientation.value
  }

  return {
    root,
    path,
    orientation,
    editor,
    editorOrientation,
    engineOn,
    engineLines,
    infinite,
    showArrows,
    nodes,
    node,
    position,
    evaluation,
    engineError,
    engineBusy,
    gameOver,
    load,
    loadPgn,
    play,
    goTo,
    back,
    forward,
    toStart,
    toEnd,
    remove,
    promote,
    pgn,
    attach,
    detach,
    analyseSetup,
    editPosition,
    origin,
    mainline,
    mainlineKey,
    review,
    gameAnalysis,
    reviewMarks,
    reviewedPlies,
    reviewWhole,
    reviewProgress,
    requestReview,
    cancelReview,
  }
})
