import { defineStore } from 'pinia'
import { computed, effectScope, ref, shallowReactive, watch, type EffectScope } from 'vue'
import { readSession, persistSession } from '../utils/sessionPersistence'
import { useLocalStorage } from '@vueuse/core'
import type { Color } from '@lichess-org/chessground/types'
import type { AnalysisUpdate, CloudEval, Judgment } from '../../src/shared/types'
import { analysisContext } from '../../src/shared/analysisContext'
import { analyseReview, reviewKey, type GameAnalysis } from '../../src/shared/review'
import { useKChessStore } from './kchess'
import { useReviewStore } from './review'
import { useStudyStore } from './studies'
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
  const app = useKChessStore()
  const saved = readSession('kchess:analysis:v1', (raw) => {
    if (!raw || typeof raw !== 'object') return undefined
    const value = raw as {
      version?: unknown
      pgn?: unknown
      path?: unknown
      orientation?: unknown
      study?: unknown
    }
    if (value.version !== 1 || typeof value.pgn !== 'string') return undefined
    const root = treeFromPgn(value.pgn)
    if (!root) return undefined
    const path =
      typeof value.path === 'string'
        ? pathOf(
            nodesAlong(root, value.path)
              .slice(1)
              .map((n) => n.uci),
          )
        : ''
    return {
      root,
      path,
      orientation: value.orientation === 'black' ? ('black' as const) : ('white' as const),
      study: typeof value.study === 'string' ? value.study : '',
    }
  })
  const root = ref<TreeNode>(saved?.root ?? newTree())
  const path = ref(saved?.path ?? '')
  const orientation = ref<Color>(saved?.orientation ?? 'white')
  /** The saved study the board is showing; its edits are saved back to it as they happen. */
  const studyId = ref(saved?.study ?? '')
  const saveError = persistSession('kchess:analysis:v1', () => ({
    version: 1,
    pgn: treeToPgn(root.value),
    path: path.value,
    orientation: orientation.value,
    study: studyId.value,
  }))
  /** The board editor's position, kept while you visit other pages. */
  const editor = ref<EditorSetup>(structuredClone(START_SETUP))
  const editorOrientation = ref<Color>('white')

  const engineOn = useLocalStorage('kchess:analysis-engine', true)
  const engineLines = useLocalStorage('kchess:analysis-lines', 3)
  const infinite = useLocalStorage('kchess:analysis-infinite', false)
  const showArrows = useLocalStorage('kchess:analysis-arrows', true)
  /** The tool shown under the engine lines. */
  const tool = useLocalStorage<'moves' | 'review' | 'explorer'>('kchess:analysis-tool', 'moves')

  const nodes = computed(() => nodesAlong(root.value, path.value))
  const node = computed(() => nodes.value.at(-1)!)
  const position = computed(() => positionFromFen(node.value.fen))

  /* ── Navigation and editing the tree ──────────────────────────────── */

  function load(fen?: string): void {
    root.value = newTree(fen)
    path.value = ''
    evaluations.clear()
    origin.value = null
    studyId.value = ''
  }
  /** Load a game; `ply` opens it at that many moves into the main line (default: the start). */
  function loadPgn(text: string, ply = 0): boolean {
    const tree = treeFromPgn(text)
    if (!tree) return false
    root.value = tree
    path.value = pathOf(movesOf(lineEnd(tree, '')).slice(0, Math.max(0, ply)))
    evaluations.clear()
    origin.value = null
    studyId.value = ''
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

  /* ── Studies ─────────────────────────────────────────────────────── */

  const studies = useStudyStore()
  const study = computed(() => studies.items.find((item) => item.id === studyId.value))
  /** The PGN last written to the open study, so opening one does not count as an edit. */
  let studyPgn = study.value?.pgn ?? ''
  const studySaveError = ref('')
  function openStudy(id: string): boolean {
    const found = studies.items.find((item) => item.id === id)
    if (!found || !loadPgn(found.pgn)) return false
    studyId.value = id
    studyPgn = pgn()
    return true
  }
  /** Keep the board as a study (named), and save its edits to it from now on. */
  function saveAsStudy(name: string): string {
    const text = pgn()
    studyId.value = studies.save(name, text)
    studyPgn = text
    return studyId.value
  }
  /** Stop saving edits to the open study; the board keeps its moves. */
  function closeStudy(): void {
    studyId.value = ''
  }
  watch(
    () => (studyId.value ? pgn() : ''),
    (text) => {
      const open = study.value
      if (!open || !text || text === studyPgn) return
      try {
        studies.save(open.name, text, open.id)
        studyPgn = text
        studySaveError.value = ''
      } catch (cause) {
        studySaveError.value = cause instanceof Error ? cause.message : String(cause)
      }
    },
  )
  // A study removed from the library (here or on the Studies page) no longer receives edits.
  watch(study, (open) => {
    if (!open && studyId.value) studyId.value = ''
  })

  /* ── Engine ──────────────────────────────────────────────────────── */

  const evaluations = shallowReactive(new Map<string, AnalysisUpdate>())
  const assistanceAllowed = computed(
    () => !['playing', 'seeking', 'disconnected'].includes(app.onlinePhase),
  )
  const live = ref<AnalysisUpdate | null>(null)
  const engineError = ref('')
  const engineBusy = ref(false)
  let wanted = ''
  let clientId = 0
  const context = (): string => analysisContext(root.value.fen, movesOf(path.value))
  let latestId = 0
  let scope: EffectScope | undefined
  let off: (() => void) | undefined

  // Settings can change while this page is detached. Invalidate in the store's lifetime scope.
  watch(
    [() => app.settings?.enginePath, () => app.engineInfo?.identity],
    () => {
      ++clientId
      wanted = ''
      evaluations.clear()
      live.value = null
      engineBusy.value = false
    },
    { flush: 'sync' },
  )

  /** The deepest evaluation known for the position on the board. */
  const evaluation = computed<AnalysisUpdate | null>(() => {
    if (!assistanceAllowed.value) return null
    const fen = node.value.fen
    if (live.value?.fen === fen && live.value.context === context()) return live.value
    return evaluations.get(context()) ?? null
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
    const key = update.context ?? update.fen
    const known = evaluations.get(key)
    if (!update.lines.length) return
    if (known && known.depth > update.depth && known.lines.length >= update.lines.length) return
    evaluations.delete(key)
    evaluations.set(key, update)
    if (evaluations.size > CACHE_SIZE) evaluations.delete(evaluations.keys().next().value!)
  }

  function receive(update: AnalysisUpdate): void {
    // Updates for a position no longer on the board (or an older request for it) are stale.
    if (
      update.fen !== wanted ||
      update.clientId !== clientId ||
      update.context !== context() ||
      update.id < latestId
    )
      return
    latestId = update.id
    live.value = update
    remember(update)
    if (update.error) {
      engineError.value = update.error
      engineBusy.value = false
      return
    }
    if (update.done) engineBusy.value = false
  }

  async function analyse(): Promise<void> {
    const fen = node.value.fen
    const requestId = ++clientId
    engineError.value = ''
    if (
      !engineOn.value ||
      gameOver.value ||
      !position.value ||
      ['playing', 'seeking', 'disconnected'].includes(app.onlinePhase) ||
      reviews.status.current?.key === mainlineKey.value
    ) {
      wanted = ''
      engineBusy.value = false
      void window.kchess.stopAnalysis().catch(() => {})
      return
    }
    const known = evaluations.get(context())
    // A finished search with enough lines needs no new one, unless analysis is meant to go on.
    if (
      !infinite.value &&
      known?.reason === 'completed' &&
      known.engine === app.engineInfo?.identity &&
      known.lines.length >= engineLines.value
    ) {
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
        clientId: requestId,
        rootFen: root.value.fen,
        moves: movesOf(path.value),
        lines: Math.max(1, Math.min(5, engineLines.value)),
        infinite: infinite.value,
      })
      if (wanted === fen && clientId === requestId) latestId = Math.max(latestId, id)
    } catch (cause) {
      if (wanted !== fen || clientId !== requestId) return
      engineBusy.value = false
      engineError.value = cause instanceof Error ? cause.message : String(cause)
    }
  }

  /* ── Lichess cloud evaluation ────────────────────────────────────── */

  /** Lichess's stored evaluation of the position, when cloud evaluation is on and it has one. */
  const cloud = ref<CloudEval | null>(null)
  const cloudBusy = ref(false)
  const cloudError = ref('')
  let cloudRequest = 0
  let cloudTimer: ReturnType<typeof setTimeout> | undefined
  function askCloud(): void {
    clearTimeout(cloudTimer)
    const fen = node.value.fen
    const request = ++cloudRequest
    if (cloud.value?.fen !== fen) cloud.value = null
    cloudError.value = ''
    if (!app.settings?.cloudEval || !assistanceAllowed.value || gameOver.value) {
      cloudBusy.value = false
      return
    }
    // Stepping quickly through a game should not send every position on the way.
    cloudTimer = setTimeout(async () => {
      cloudBusy.value = true
      try {
        const result = await window.kchess.cloudEval(
          fen,
          Math.max(1, Math.min(5, engineLines.value)),
        )
        if (request === cloudRequest) cloud.value = result
      } catch (cause) {
        if (request === cloudRequest)
          cloudError.value = cause instanceof Error ? cause.message : String(cause)
      } finally {
        if (request === cloudRequest) cloudBusy.value = false
      }
    }, 300)
  }

  /** Start following the board with the engine (the analysis page is open). */
  function attach(): void {
    if (scope) return
    off = window.kchess.onAnalysis(receive)
    scope = effectScope()
    scope.run(() => {
      watch(
        () =>
          [node.value.fen, app.settings?.cloudEval, engineLines.value, app.onlinePhase] as const,
        () => askCloud(),
        { immediate: true },
      )
      watch(
        () =>
          [
            root.value.fen,
            path.value,
            engineOn.value,
            engineLines.value,
            infinite.value,
            app.settings.enginePath,
            app.engineInfo?.identity,
            app.onlinePhase,
            reviews.status.current?.key,
          ] as const,
        () => void analyse(),
        { immediate: true },
      )
    })
  }
  function detach(): void {
    clearTimeout(cloudTimer)
    cloudRequest++
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
    assistanceAllowed.value && review.value && review.value.fen === root.value.fen
      ? analyseReview(review.value)
      : null,
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
    tool.value = 'review'
    if (!mainline.value.length) return
    wanted = ''
    await window.kchess.stopAnalysis()
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
    saveError,
    root,
    path,
    orientation,
    editor,
    editorOrientation,
    engineOn,
    engineLines,
    infinite,
    showArrows,
    tool,
    nodes,
    node,
    position,
    evaluation,
    cloud,
    cloudBusy,
    cloudError,
    assistanceAllowed,
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
    studyId,
    study,
    studySaveError,
    openStudy,
    saveAsStudy,
    closeStudy,
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
