import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import { pickConnectedAccount } from '../../src/shared/accounts'
import type {
  LichessRatingHistory,
  LichessUser,
  NeedsReconnect,
  Puzzle,
  PuzzleActivityEntry,
  PuzzleDashboard,
  PuzzleDbProgress,
  PuzzleDbStatus,
  PuzzleDifficulty,
  StormDashboard,
} from '../../src/shared/types'
import { useKChessStore } from './kchess'

export type PuzzleTab = 'train' | 'daily' | 'rush' | 'stats' | 'history'

/**
 * How a training session relates to Lichess:
 * `rated` — results are sent to Lichess and change the puzzle rating;
 * `practice` — Lichess puzzles, nothing is sent;
 * `offline` — puzzles from the local database, nothing is sent.
 */
export type TrainMode = 'rated' | 'practice' | 'offline'

export type TrainPhase = 'idle' | 'loading' | 'ready' | 'reconnect' | 'noaccount' | 'nodb' | 'error'

const isReconnect = (value: unknown): value is NeedsReconnect =>
  typeof value === 'object' && value !== null && 'needsReconnect' in value

/** Where each difficulty aims, relative to your rating, when puzzles come from the local database. */
const OFFLINE_OFFSET: Record<PuzzleDifficulty, number> = {
  easiest: -600,
  easier: -300,
  normal: 0,
  harder: 300,
  hardest: 600,
}

const message = (cause: unknown): string => (cause instanceof Error ? cause.message : String(cause))

export const usePuzzleStore = defineStore('puzzles', () => {
  const kchess = useKChessStore()

  const tab = useLocalStorage<PuzzleTab>('kchess:puzzle-tab', 'train')
  const angle = useLocalStorage('kchess:puzzle-angle', 'mix')
  const difficulty = useLocalStorage<PuzzleDifficulty>('kchess:puzzle-difficulty', 'normal')
  const color = useLocalStorage<'random' | 'white' | 'black'>('kchess:puzzle-color', 'random')
  const mode = useLocalStorage<TrainMode>('kchess:puzzle-mode', 'rated')
  const accountPreference = useLocalStorage('kchess:puzzle-account', '')

  const account = computed(
    () => pickConnectedAccount(kchess.data.accounts, accountPreference.value)?.username ?? '',
  )
  const setAccount = (username: string): void => {
    accountPreference.value = username
  }

  // ── Training ──────────────────────────────────────────────────────────────────────────────────
  const phase = ref<TrainPhase>('idle')
  const current = ref<Puzzle | null>(null)
  /** Bumps for every new puzzle, so the board starts fresh even for a puzzle shown twice. */
  const puzzleKey = ref(0)
  /** A puzzle opened from history: solving it is local only and does not touch the session. */
  const isRetry = ref(false)
  const trainError = ref('')
  /** The account's puzzle rating as Lichess last said it, moved by every rated result. */
  const rating = ref<number | undefined>()
  const session = ref({ solved: 0, failed: 0, ratingChange: 0 })
  /** What Lichess did with the last rated result. */
  const lastReport = ref<{ win: boolean; ratingDiff?: number; failed?: string } | null>(null)
  const attemptOutcome = ref<boolean | null>(null)
  let attempt: { key: number; account: string; angle: string; mode: TrainMode } | null = null
  let loadToken = 0
  let sessionVersion = 0

  const syncsToLichess = computed(
    () => mode.value === 'rated' && !isRetry.value && Boolean(account.value),
  )

  async function loadNext(): Promise<void> {
    const token = ++loadToken
    phase.value = 'loading'
    trainError.value = ''
    lastReport.value = null
    isRetry.value = false
    try {
      if (mode.value === 'offline') {
        const centre = (rating.value ?? 1500) + OFFLINE_OFFSET[difficulty.value]
        const [puzzle] = await window.kchess.localPuzzles({
          theme: angle.value,
          minRating: Math.max(0, centre - 150),
          maxRating: centre + 150,
          count: 1,
        })
        if (token !== loadToken) return
        if (!puzzle)
          throw new Error('No puzzle of that theme and difficulty in the local database.')
        show(puzzle)
        return
      }
      if (mode.value === 'rated' && !account.value) {
        phase.value = 'noaccount'
        return
      }
      const draw = await window.kchess.puzzleNext({
        account: account.value,
        angle: angle.value,
        difficulty: difficulty.value,
        color: color.value === 'random' ? undefined : color.value,
      })
      if (token !== loadToken) return
      if (isReconnect(draw)) {
        if (mode.value === 'rated') {
          phase.value = 'reconnect'
          return
        }
        // Practice does not need the permission: take a puzzle without the account.
        const anonymous = await window.kchess.puzzleNext({
          account: '',
          angle: angle.value,
          difficulty: difficulty.value,
          color: color.value === 'random' ? undefined : color.value,
        })
        if (token !== loadToken) return
        if (!isReconnect(anonymous)) show(anonymous.puzzle)
        return
      }
      if (draw.glicko?.rating !== undefined) rating.value = Math.round(draw.glicko.rating)
      show(draw.puzzle)
    } catch (cause) {
      if (token !== loadToken) return
      const text = message(cause)
      if (mode.value === 'offline' && /Download the puzzle database/.test(text))
        phase.value = 'nodb'
      else {
        trainError.value = text
        phase.value = 'error'
      }
    }
  }

  function show(puzzle: Puzzle): void {
    current.value = puzzle
    puzzleKey.value++
    attemptOutcome.value = null
    attempt = {
      key: puzzleKey.value,
      account: account.value,
      angle: angle.value,
      mode: mode.value,
    }
    phase.value = 'ready'
  }

  /** Work on a puzzle from the history list; nothing about it is sent to Lichess. */
  function retry(puzzle: Puzzle): void {
    ++loadToken
    isRetry.value = true
    lastReport.value = null
    show(puzzle)
    tab.value = 'train'
  }

  /** Start training on one theme, from anywhere (the practice page, the stats tab). */
  function startTheme(theme: string, nextMode?: TrainMode): void {
    angle.value = theme
    if (nextMode) mode.value = nextMode
    tab.value = 'train'
    // The trainer loads a puzzle when it opens (or when its inputs change), so this asks for no request itself.
    ++loadToken
    phase.value = 'idle'
    void navigateTo('/puzzles')
  }

  /** The first attempt on the current puzzle has been decided. */
  async function report(win: boolean): Promise<void> {
    const puzzle = current.value
    const started = attempt
    if (
      !puzzle ||
      !started ||
      isRetry.value ||
      phase.value !== 'ready' ||
      attemptOutcome.value !== null ||
      started.account !== account.value ||
      started.mode !== mode.value ||
      started.angle !== angle.value
    )
      return
    // The verdict belongs to the store's attempt, so remounting the board cannot submit it again.
    attemptOutcome.value = win
    if (win) session.value.solved++
    else session.value.failed++
    if (!syncsToLichess.value) return
    const token = loadToken
    const version = sessionVersion
    const stillCurrent = (): boolean =>
      token === loadToken &&
      version === sessionVersion &&
      started === attempt &&
      started.account === account.value &&
      started.mode === mode.value &&
      started.angle === angle.value
    try {
      const result = await window.kchess.puzzleSolve({
        account: started.account,
        angle: started.angle,
        id: puzzle.id,
        win,
        rated: true,
      })
      if (!stillCurrent()) return
      if (isReconnect(result)) {
        phase.value = 'reconnect'
        lastReport.value = {
          win,
          failed: 'Lichess did not accept the result: reconnect your account.',
        }
        return
      }
      if (result.ratingDiff !== undefined) {
        session.value.ratingChange += result.ratingDiff
        if (rating.value !== undefined) rating.value += result.ratingDiff
      }
      if (current.value?.id === puzzle.id) lastReport.value = { win, ratingDiff: result.ratingDiff }
    } catch (cause) {
      if (!stillCurrent()) return
      lastReport.value = { win, failed: message(cause) }
    }
  }

  function resetSession(): void {
    sessionVersion++
    session.value = { solved: 0, failed: 0, ratingChange: 0 }
  }

  // ── Daily puzzle ──────────────────────────────────────────────────────────────────────────────
  const daily = ref<Puzzle | null>(null)
  const dailyError = ref('')
  const dailyLoading = ref(false)
  async function loadDaily(force = false): Promise<void> {
    if (daily.value && !force) return
    dailyLoading.value = true
    dailyError.value = ''
    try {
      daily.value = await window.kchess.puzzleDaily()
    } catch (cause) {
      dailyError.value = message(cause)
    } finally {
      dailyLoading.value = false
    }
  }

  // ── Read-only data from Lichess ───────────────────────────────────────────────────────────────
  const profile = ref<LichessUser | null>(null)
  const ratingHistory = ref<LichessRatingHistory>([])
  const dashboard = ref<PuzzleDashboard | null>(null)
  const dashboardDays = useLocalStorage('kchess:puzzle-dashboard-days', 30)
  const storm = ref<StormDashboard | null>(null)
  const stats = ref({ loading: false, error: '', needsReconnect: false, account: '' })
  let statsToken = 0

  async function loadStats(): Promise<void> {
    const token = ++statsToken
    const who = account.value
    if (!who) {
      profile.value = null
      ratingHistory.value = []
      dashboard.value = null
      storm.value = null
      stats.value = { loading: false, error: '', needsReconnect: false, account: '' }
      return
    }
    stats.value = { loading: true, error: '', needsReconnect: false, account: who }
    const [user, history, board, runs] = await Promise.allSettled([
      window.kchess.profile(who),
      window.kchess.ratingHistory(who),
      window.kchess.puzzleDashboard(who, dashboardDays.value),
      window.kchess.stormDashboard(who, 30),
    ])
    if (token !== statsToken || who !== account.value) return
    profile.value = user.status === 'fulfilled' ? user.value : null
    if (user.status === 'fulfilled' && user.value.perfs?.puzzle?.rating !== undefined)
      rating.value ??= Math.round(user.value.perfs.puzzle.rating)
    ratingHistory.value = history.status === 'fulfilled' ? history.value : []
    storm.value = runs.status === 'fulfilled' ? runs.value : null
    let needsReconnect = false
    let error = ''
    if (board.status === 'fulfilled') {
      if (isReconnect(board.value)) {
        dashboard.value = null
        needsReconnect = true
      } else dashboard.value = board.value
    } else {
      dashboard.value = null
      error = message(board.reason)
    }
    stats.value = { loading: false, error, needsReconnect, account: who }
  }

  const activity = ref<PuzzleActivityEntry[]>([])
  const activityState = ref({ loading: false, error: '', needsReconnect: false, loaded: '' })
  let activityToken = 0
  async function loadActivity(force = false): Promise<void> {
    const who = account.value
    if (!who || (!force && activityState.value.loaded === who)) return
    const token = ++activityToken
    activityState.value = { loading: true, error: '', needsReconnect: false, loaded: '' }
    try {
      const result = await window.kchess.puzzleActivity(who, 100)
      if (token !== activityToken || who !== account.value) return
      if (isReconnect(result)) {
        activityState.value = { loading: false, error: '', needsReconnect: true, loaded: '' }
        return
      }
      activity.value = result
      activityState.value = { loading: false, error: '', needsReconnect: false, loaded: who }
    } catch (cause) {
      if (token !== activityToken || who !== account.value) return
      activityState.value = {
        loading: false,
        error: message(cause),
        needsReconnect: false,
        loaded: '',
      }
    }
  }

  watch(
    account,
    () => {
      ++loadToken
      ++statsToken
      ++activityToken
      attempt = null
      attemptOutcome.value = null
      phase.value = 'idle'
      rating.value = undefined
      lastReport.value = null
      resetSession()
      profile.value = null
      ratingHistory.value = []
      dashboard.value = null
      storm.value = null
      stats.value = { loading: false, error: '', needsReconnect: false, account: account.value }
      activity.value = []
      activityState.value = { loading: false, error: '', needsReconnect: false, loaded: '' }
    },
    { flush: 'sync' },
  )

  /** Connecting again grants the puzzle permission that older logins lack. */
  async function reconnect(): Promise<void> {
    await kchess.connect()
    activityState.value.loaded = ''
    if (phase.value === 'reconnect') void loadNext()
    if (tab.value === 'stats') void loadStats()
    if (tab.value === 'history') void loadActivity(true)
  }

  // ── Local puzzle database ─────────────────────────────────────────────────────────────────────
  const db = ref<PuzzleDbStatus | null>(null)
  const dbProgress = ref<PuzzleDbProgress | null>(null)
  const dbError = ref('')
  let offProgress: (() => void) | undefined

  async function refreshDb(): Promise<void> {
    try {
      db.value = await window.kchess.puzzleDbStatus()
    } catch (cause) {
      dbError.value = message(cause)
    }
    offProgress ??= window.kchess.onPuzzleDbProgress((progress) => {
      dbProgress.value = progress
    })
  }
  async function installDb(): Promise<void> {
    dbError.value = ''
    dbProgress.value = { phase: 'downloading', received: 0, kept: 0 }
    try {
      db.value = await window.kchess.puzzleDbInstall()
    } catch (cause) {
      dbError.value = message(cause)
    } finally {
      await refreshDb()
      if (dbProgress.value?.phase === 'done' || dbProgress.value?.phase === 'cancelled')
        dbProgress.value = null
    }
  }
  const cancelDb = (): Promise<void> => window.kchess.puzzleDbCancel()
  async function deleteDb(): Promise<void> {
    db.value = await window.kchess.puzzleDbDelete()
    dbProgress.value = null
  }
  const dbBusy = computed(
    () =>
      db.value?.busy === true ||
      dbProgress.value?.phase === 'downloading' ||
      dbProgress.value?.phase === 'importing',
  )

  return {
    tab,
    angle,
    difficulty,
    color,
    mode,
    account,
    setAccount,
    phase,
    current,
    puzzleKey,
    attemptOutcome,
    isRetry,
    trainError,
    rating,
    session,
    lastReport,
    syncsToLichess,
    loadNext,
    retry,
    startTheme,
    report,
    resetSession,
    daily,
    dailyError,
    dailyLoading,
    loadDaily,
    profile,
    ratingHistory,
    dashboard,
    dashboardDays,
    storm,
    stats,
    loadStats,
    activity,
    activityState,
    loadActivity,
    reconnect,
    db,
    dbProgress,
    dbError,
    dbBusy,
    refreshDb,
    installDb,
    cancelDb,
    deleteDb,
  }
})
