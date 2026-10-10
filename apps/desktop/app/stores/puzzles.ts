import { defineStore } from 'pinia'
import { computed, reactive, ref, toRefs, watch, onScopeDispose } from 'vue'
import { useLocalStorage, useOnline } from '@vueuse/core'
import { pickConnectedAccount } from '@kchess/rules/accounts'
import type {
  LichessRatingHistory,
  LichessUser,
  Puzzle,
  PuzzleActivityEntry,
  PuzzleDashboard,
  PuzzleDbProgress,
  PuzzleDbStatus,
  PuzzleDifficulty,
  StormDashboard,
} from '@kchess/contracts/types'
import {
  PuzzleSession,
  puzzleSessionState,
  isReconnect,
  type TrainMode,
} from '@kchess/rules/puzzleSession'
import { useKChessStore } from './kchess'
export type { TrainMode, TrainPhase } from '@kchess/rules/puzzleSession'

export type PuzzleTab = 'train' | 'daily' | 'rush' | 'stats' | 'history'

const message = (cause: unknown): string => (cause instanceof Error ? cause.message : String(cause))

export const usePuzzleStore = defineStore('puzzles', () => {
  const kchess = useKChessStore()

  const tab = useLocalStorage<PuzzleTab>('kchess:puzzle-tab', 'train')
  const angle = useLocalStorage('kchess:puzzle-angle', 'mix')
  const difficulty = useLocalStorage<PuzzleDifficulty>('kchess:puzzle-difficulty', 'normal')
  const color = useLocalStorage<'random' | 'white' | 'black'>('kchess:puzzle-color', 'random')
  const online = useOnline()
  // Remember explicit choices; new users start with account-free training.
  const mode = useLocalStorage<TrainMode>(
    'kchess:puzzle-mode',
    online.value ? 'practice' : 'offline',
  )
  const accountPreference = useLocalStorage('kchess:puzzle-account', '')

  const account = computed(
    () => pickConnectedAccount(kchess.data.accounts, accountPreference.value)?.username ?? '',
  )
  const setAccount = (username: string): void => {
    accountPreference.value = username
  }

  const training = reactive(puzzleSessionState())
  const {
    phase,
    current,
    puzzleKey,
    isRetry,
    trainError,
    rating,
    session,
    lastReport,
    attemptOutcome,
  } = toRefs(training)
  const trainer = new PuzzleSession(training, {
    api: window.kchess,
    selection: () => ({
      account: account.value,
      angle: angle.value,
      difficulty: difficulty.value,
      color: color.value,
      mode: mode.value,
    }),
    online: () => online.value,
  })
  const syncsToLichess = computed(() => trainer.syncsToLichess)
  const loadNext = (): Promise<void> => trainer.loadNext()
  const report = (win: boolean): Promise<void> => trainer.report(win)
  const resetSession = (): void => trainer.resetSession()
  const retry = (puzzle: Puzzle): void => {
    trainer.retry(puzzle)
    tab.value = 'train'
  }
  function startTheme(theme: string, nextMode?: TrainMode): void {
    angle.value = theme
    if (nextMode) mode.value = nextMode
    tab.value = 'train'
    trainer.invalidate()
    void navigateTo('/puzzles')
  }
  onScopeDispose(() => trainer.invalidate())

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
      console.warn('[puzzles] loading daily puzzle failed:', cause)
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
      console.warn('[puzzles] loading puzzle activity failed:', cause)
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
      trainer.invalidate()
      ++statsToken
      ++activityToken
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
      console.warn('[puzzles] reading puzzle database status failed:', cause)
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
      console.warn('[puzzles] installing puzzle database failed:', cause)
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
