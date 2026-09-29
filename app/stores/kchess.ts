import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import { useDebounceFn, useIntervalFn, useLocalStorage, useMediaQuery } from '@vueuse/core'
import type { DropdownMenuItem, NavigationMenuItem } from '@nuxt/ui'
import { pickConnectedAccount } from '../../src/shared/accounts'
import { canPlayOnline, perfFor } from '../../src/shared/timeControl'
import { isGameInProgress } from '../../src/shared/gameStatus'
import { boardThemes } from '../utils/boards'
import { allThemes, applyTheme, findTheme } from '../utils/themes'
import {
  checkColor,
  destsFor,
  drawReason,
  fen,
  lastMoveKeys,
  navigatePly,
  pgnFromMoves,
  playUci,
  positionAfter,
  sanHistory,
  statusText,
  takebackMoves,
  turnColor,
} from '../utils/chess'
import type { PlayerPresence } from '../components/PlayerLine.vue'
import { Clock, formatClock } from '../utils/clock'
import { configure, play, playMoveSound } from '../utils/sound'
import { formatGameDate, gameResult } from '../utils/games'
import { mergeRatingHistories, ratingHistoryFromGames } from '../utils/ratings'
import { formatBytes, signalFromLatency } from '../utils/format'
import { useUsageStore } from './usage'
import type {
  AppData,
  ChallengeColor,
  EngineLevel,
  EngineStatus,
  LichessGame,
  LichessRatingHistory,
  LichessUser,
  OnlineEvent,
  AppTheme,
  NotificationKind,
  PresenceReport,
  Settings,
} from '../../src/shared/types'

export type Page =
  'dashboard' | 'online' | 'computer' | 'puzzles' | 'practice' | 'history' | 'friends' | 'settings'

/** Sections of the Settings page; while it is open they replace the sidebar's pages. */
export const SETTINGS_SECTIONS = [
  { id: 'appearance', label: 'Appearance', icon: 'i-lucide-palette' },
  { id: 'themes', label: 'App theme', icon: 'i-lucide-swatch-book' },
  { id: 'gameplay', label: 'Gameplay', icon: 'i-lucide-gamepad-2' },
  { id: 'sound', label: 'Sound', icon: 'i-lucide-volume-2' },
  { id: 'notifications', label: 'Notifications', icon: 'i-lucide-bell' },
  { id: 'engine', label: 'Chess engine', icon: 'i-lucide-cpu' },
  { id: 'accounts', label: 'My accounts', icon: 'i-lucide-user-round' },
  { id: 'data', label: 'Data & storage', icon: 'i-lucide-database' },
] as const
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]['id']

export const useKChessStore = defineStore('kchess', () => {
  /** Pages in the sidebar. Settings is reached from the account menu instead. */
  const nav: { id: Page; label: string; icon: string }[] = [
    { id: 'dashboard', label: 'Dashboard', icon: 'i-lucide-layout-dashboard' },
    { id: 'online', label: 'Play Online', icon: 'i-lucide-globe-2' },
    { id: 'computer', label: 'Play with Computer', icon: 'i-lucide-monitor' },
    { id: 'puzzles', label: 'Puzzles', icon: 'i-lucide-puzzle' },
    { id: 'practice', label: 'Practice', icon: 'i-lucide-graduation-cap' },
    { id: 'history', label: 'History', icon: 'i-lucide-history' },
    { id: 'friends', label: 'Friends', icon: 'i-lucide-users' },
  ]
  const route = useRoute()
  const page = computed<Page>(() => {
    const section = route.path.slice(1)
    return ['online', 'computer', 'puzzles', 'practice', 'history', 'friends', 'settings'].includes(
      section,
    )
      ? (section as Page)
      : 'dashboard'
  })
  /** Where "Back" in the settings sidebar returns to. */
  const backTarget = ref<Page>('dashboard')
  watch(page, (next, previous) => {
    if (next === 'settings' && previous && previous !== 'settings') backTarget.value = previous
  })
  const settingsSection = ref<SettingsSection>('appearance')
  const usage = useUsageStore()
  const sidebarOpen = useLocalStorage('kchess:sidebar-open', true)
  const isNarrow = useMediaQuery('(max-width: 1023px)')
  const searchOpen = ref(false)
  const toast = useToast()
  const data = ref<AppData | null>(null)
  const settings = ref<Settings | null>(null)
  const busy = ref(false)
  const message = ref('')
  const error = ref('')
  const usernameInput = ref('')
  const selectedAccount = ref('')
  const profile = ref<LichessUser | null>(null)
  const ratingHistories = ref<LichessRatingHistory>([])
  const chartMode = ref('Blitz')
  const chartRange = ref('All')
  const historyResult = ref('all')
  const historyRated = ref('all')
  const historyAccount = ref('all')
  const historyPage = ref(0)
  const historyPageSize = ref(20)
  const reviewGame = ref<LichessGame | null>(null)
  const localMoves = ref<string[]>([])
  const localPly = ref(0)
  const level = ref<EngineLevel>('medium')
  const userColor = ref<'white' | 'black'>('white')
  const flipped = ref(false)
  const thinking = ref(false)
  const gameEpoch = ref(0)
  const engineReady = ref(false)
  /** True until the first engine check lands, and while a re-check runs. */
  const engineChecking = ref(true)
  const engineInfo = ref<EngineStatus | null>(null)
  /** A yes/no question raised outside any page (e.g. from the search palette). */
  const confirmation = ref<{
    title: string
    description: string
    label: string
    run: () => void
  } | null>(null)
  const onlinePhase = ref<'idle' | 'seeking' | 'playing' | 'finished'>('idle')
  const onlineId = ref('')
  const onlineMoves = ref<string[]>([])
  const onlineColor = ref<'white' | 'black'>('white')
  const onlineOpponent = ref('Opponent')
  const onlineStatus = ref('')
  const onlineInitial = ref(0)
  const onlineMinutes = ref(15)
  const onlineIncrement = ref(10)
  const onlineTarget = ref('')
  const onlineChoice = ref<ChallengeColor>('random')
  /** The "rated" choice in the find-a-game form. */
  const onlineRated = ref(false)
  /** Whether the game being played is rated, as Lichess reports it. */
  const onlineGameRated = ref(false)
  /** Position being looked at while reviewing a live game; null follows the game. */
  const onlineViewPly = ref<number | null>(null)
  const onlineFlipped = ref(false)
  const opponentId = ref('')
  const opponentGone = ref(false)
  const presence = ref<PresenceReport | null>(null)
  /** Recent round trips to Lichess, oldest first; the game panel graphs them. */
  const latencySamples = ref<number[]>([])
  /** How long Lichess took to accept our last move. */
  const moveAckMs = ref<number | null>(null)
  const clock = new Clock()
  /** Ask the main process for a desktop notification; it applies the Settings and window-state rules. */
  function notifyDesktop(kind: NotificationKind, title: string, body: string): void {
    void window.kchess.notify({ kind, title, body }).catch(() => undefined)
  }
  let offOnline: (() => void) | undefined
  let offOnlineError: (() => void) | undefined
  let offNotification: (() => void) | undefined
  let initialized = false
  /** Drives the on-screen clocks and the low-time alert; runs only while the app is initialised. */
  const ticker = useIntervalFn(
    () => {
      now.value = performance.now()
      if (onlinePhase.value === 'playing') {
        const turn = onlineTurn.value
        if (clock.lowTimeAlert(turn)) {
          void play('lowTime')
          if (turn === onlineColor.value)
            notifyDesktop(
              'lowTime',
              'Low on time',
              `${formatClock(clock.remaining(turn))} left against ${onlineOpponent.value}.`,
            )
        }
      }
    },
    250,
    { immediate: false },
  )

  const localGame = computed(() => positionAfter(localMoves.value))
  const localDraw = computed(() => drawReason(localMoves.value))
  const localOver = computed(() => localGame.value.isEnd() || Boolean(localDraw.value))
  const localDisplay = computed(() => positionAfter(localMoves.value.slice(0, localPly.value)))
  const localHistory = computed(() => sanHistory(localMoves.value))
  const localLast = computed(() => lastMoveKeys(localMoves.value.slice(0, localPly.value)))
  const localDests = computed(() => destsFor(localDisplay.value))
  const localTurn = computed(() => turnColor(localDisplay.value))
  const localCheck = computed(() => checkColor(localDisplay.value))
  /** How a finished computer game ended, from the player's point of view. */
  const localResult = computed<{
    kind: 'win' | 'loss' | 'draw'
    title: string
    detail: string
  } | null>(() => {
    if (!localOver.value) return null
    const game = localGame.value
    if (game.isCheckmate()) {
      const won = game.turn !== userColor.value
      return {
        kind: won ? 'win' : 'loss',
        title: won ? 'You won' : 'Stockfish won',
        detail: 'Checkmate',
      }
    }
    const detail = localDraw.value ?? (game.isStalemate() ? 'Stalemate' : 'Insufficient material')
    return { kind: 'draw', title: 'Draw', detail }
  })
  /** The player may touch pieces: it is their move, or they may queue one. */
  const localCanPlay = computed(
    () => engineReady.value && !localOver.value && localPly.value === localMoves.value.length,
  )
  const localInteractive = computed(() => localCanPlay.value && localTurn.value === userColor.value)

  const onlinePosition = computed(() => positionAfter(onlineMoves.value))
  const onlineHistory = computed(() => sanHistory(onlineMoves.value))
  const onlinePly = computed(() =>
    Math.min(onlineViewPly.value ?? Infinity, onlineMoves.value.length),
  )
  const onlineAtLive = computed(() => onlinePly.value === onlineMoves.value.length)
  /** The position on the board: the live one, or an earlier one while reviewing. */
  const onlineDisplay = computed(() =>
    onlineAtLive.value
      ? onlinePosition.value
      : positionAfter(onlineMoves.value.slice(0, onlinePly.value)),
  )
  const onlineLast = computed(() => lastMoveKeys(onlineMoves.value.slice(0, onlinePly.value)))
  const onlineDests = computed(() => destsFor(onlinePosition.value))
  /** Whose turn it really is (drives the clocks); the board's turn is `onlineDisplayTurn`. */
  const onlineTurn = computed(() => turnColor(onlinePosition.value))
  const onlineDisplayTurn = computed(() => turnColor(onlineDisplay.value))
  const onlineCheck = computed(() => checkColor(onlineDisplay.value))
  /** The player may touch pieces: it is their move, or they may queue one. */
  const onlineCanPlay = computed(() => onlinePhase.value === 'playing' && onlineAtLive.value)
  const onlineInteractive = computed(
    () => onlineCanPlay.value && onlineTurn.value === onlineColor.value,
  )
  function viewOnlinePly(ply: number): void {
    const clamped = Math.max(0, Math.min(ply, onlineMoves.value.length))
    onlineViewPly.value = clamped === onlineMoves.value.length ? null : clamped
  }
  const onlineOrientation = computed(() =>
    onlineFlipped.value ? (onlineColor.value === 'white' ? 'black' : 'white') : onlineColor.value,
  )

  /** Lichess's view of each side's connection, for the player lines. */
  const onlinePresence = computed<{ self?: PlayerPresence; opponent?: PlayerPresence }>(() => {
    if (onlinePhase.value !== 'playing') return {}
    const report = presence.value
    const me = report?.users[activeOnlineAccount.value.toLowerCase()]
    const them = report?.users[opponentId.value.toLowerCase()]
    const ping = pingStats.value
    return {
      self: ping
        ? {
            state: me?.online === false ? 'offline' : 'online',
            signal: me?.signal ?? signalFromLatency(ping.current),
            latencyMs: ping.current,
          }
        : { state: 'checking', label: 'Measuring…' },
      opponent: opponentGone.value
        ? { state: 'away', signal: undefined }
        : them
          ? { state: them.online ? 'online' : 'offline', signal: them.signal }
          : { state: 'checking', label: 'Checking…' },
    }
  })
  /** Summary of the recent round trips to Lichess, or null before the first one. */
  const pingStats = computed(() => {
    const samples = latencySamples.value
    if (!samples.length) return null
    const current = samples[samples.length - 1]!
    const average = Math.round(samples.reduce((sum, ms) => sum + ms, 0) / samples.length)
    return {
      current,
      average,
      min: Math.min(...samples),
      max: Math.max(...samples),
      samples,
      quality: signalFromLatency(current),
    }
  })
  /** How often the connection is measured while a game is on; Lichess asks for about once per 5 s at most per check, this stays under that. */
  const PRESENCE_INTERVAL_MS = 3000
  async function pollPresence(): Promise<void> {
    const me = activeOnlineAccount.value
    if (onlinePhase.value !== 'playing' || !me) return
    try {
      // Measure even before the opponent is known, so the ping shows from the first second.
      const ids = opponentId.value ? [me, opponentId.value] : [me]
      const report = await window.kchess.presence(ids)
      presence.value = report
      latencySamples.value = [...latencySamples.value, report.latencyMs].slice(-30)
    } catch {
      // A missed poll just leaves the last reading up; the next one recovers.
    }
  }
  const presenceTicker = useIntervalFn(() => void pollPresence(), PRESENCE_INTERVAL_MS, {
    immediate: false,
  })
  watch(
    onlinePhase,
    (phase) => {
      if (phase === 'playing') {
        presenceTicker.resume()
        void pollPresence()
      } else {
        presenceTicker.pause()
        presence.value = null
        latencySamples.value = []
        moveAckMs.value = null
        opponentGone.value = false
      }
    },
    { immediate: true },
  )
  watch(opponentId, () => void pollPresence())

  const reviewMoves = computed(() => reviewGame.value?.moves.split(/\s+/).filter(Boolean) ?? [])
  const reviewPgn = computed(() => reviewGame.value?.pgn?.trim() || pgnFromMoves(reviewMoves.value))

  const games = computed(() => data.value?.games ?? [])
  const filteredGames = computed(() =>
    games.value.filter((game) => {
      if (historyAccount.value !== 'all' && game.account !== historyAccount.value) return false
      if (historyRated.value !== 'all' && (game.rated ? 'rated' : 'casual') !== historyRated.value)
        return false
      return historyResult.value === 'all' || gameResult(game) === historyResult.value
    }),
  )
  const historyPageCount = computed(() =>
    Math.max(1, Math.ceil(filteredGames.value.length / historyPageSize.value)),
  )
  const visibleGames = computed(() =>
    filteredGames.value.slice(
      historyPage.value * historyPageSize.value,
      (historyPage.value + 1) * historyPageSize.value,
    ),
  )
  const recentGames = computed(() =>
    games.value.filter((g) => g.account === selectedAccount.value).slice(0, 6),
  )
  const counts = computed(() => {
    const own = games.value.filter((g) => g.account === selectedAccount.value)
    return {
      win: own.filter((g) => gameResult(g) === 'win').length,
      loss: own.filter((g) => gameResult(g) === 'loss').length,
      draw: own.filter((g) => gameResult(g) === 'draw').length,
    }
  })
  /**
   * Lichess's rating history, completed from synced games where it has none
   * (some accounts return an empty history despite thousands of rated games).
   */
  const ratingHistoriesResolved = computed(() =>
    mergeRatingHistories(
      ratingHistories.value,
      ratingHistoryFromGames(games.value.filter((game) => game.account === selectedAccount.value)),
    ),
  )
  /** The chosen mode's points come from synced games rather than Lichess. */
  const chartFromGames = computed(
    () =>
      !ratingHistories.value.find((item) => item.name === chartMode.value)?.points?.length &&
      Boolean(
        ratingHistoriesResolved.value.find((item) => item.name === chartMode.value)?.points?.length,
      ),
  )
  // Keep the selected mode on one that has data.
  watch(ratingHistoriesResolved, (history) => {
    if (!history.find((item) => item.name === chartMode.value)?.points?.length)
      chartMode.value = history.find((item) => item.points?.length)?.name ?? 'Blitz'
  })
  /** Rating points for the chosen mode and range, oldest first. */
  const chartSeries = computed(() => {
    const cutoffDays: Record<string, number> = { '1M': 30, '3M': 90, '6M': 180, '1Y': 365 }
    const cutoff =
      chartRange.value === 'YTD'
        ? new Date(new Date().getFullYear(), 0, 1).getTime()
        : chartRange.value === 'All'
          ? 0
          : Date.now() - (cutoffDays[chartRange.value] ?? 0) * 86_400_000
    const points =
      ratingHistoriesResolved.value.find((item) => item.name === chartMode.value)?.points ?? []
    return points
      .filter((p) => p.length >= 4)
      .map((p) => ({ date: Date.UTC(p[0]!, p[1]!, p[2]!), rating: p[3]! }))
      .filter((point) => point.date >= cutoff)
  })
  const chartValues = computed(() => chartSeries.value.map((point) => point.rating))

  const date = formatGameDate
  function info(text: string): void {
    message.value = text
    error.value = ''
    toast.add({ title: text, icon: 'i-lucide-circle-check', color: 'success', duration: 3500 })
  }
  function fail(cause: unknown): void {
    error.value = cause instanceof Error ? cause.message : String(cause)
    message.value = ''
    toast.add({
      title: 'Something went wrong',
      description: error.value,
      icon: 'i-lucide-circle-alert',
      color: 'error',
      duration: 8000,
    })
  }
  async function run<T>(action: () => Promise<T>): Promise<T | undefined> {
    busy.value = true
    error.value = ''
    try {
      return await action()
    } catch (cause) {
      fail(cause)
      return undefined
    } finally {
      busy.value = false
    }
  }
  function selectPage(next: Page): void {
    void navigateTo(next === 'dashboard' ? '/' : `/${next}`)
    error.value = ''
    message.value = ''
  }
  function choosePage(next: Page): void {
    selectPage(next)
    if (isNarrow.value) sidebarOpen.value = false
  }
  /** Show one settings category (the sidebar lists them while Settings is open), from the top. */
  function jumpToSection(id: SettingsSection): void {
    settingsSection.value = id
    document.querySelector('.main-area')?.scrollTo({ top: 0 })
    if (isNarrow.value) sidebarOpen.value = false
  }
  // While Settings is open the sidebar lists its categories, with a way back.
  const navItems = computed<NavigationMenuItem[]>(() =>
    page.value === 'settings'
      ? [
          {
            label: 'Back',
            icon: 'i-lucide-arrow-left',
            onSelect: () => choosePage(backTarget.value),
          },
          { label: 'Settings', type: 'label' as const },
          ...SETTINGS_SECTIONS.map((section) => ({
            label: section.label,
            icon: section.icon,
            active: settingsSection.value === section.id,
            onSelect: () => jumpToSection(section.id),
          })),
        ]
      : nav.map((item) => ({
          label: item.label,
          icon: item.icon,
          active: page.value === item.id,
          onSelect: () => choosePage(item.id),
        })),
  )

  function ask(question: NonNullable<typeof confirmation.value>): void {
    confirmation.value = question
  }
  function startComputerGame(chosenLevel: EngineLevel, color: 'white' | 'black'): void {
    searchOpen.value = false
    const begin = (): void => {
      level.value = chosenLevel
      userColor.value = color
      newGame()
      selectPage('computer')
    }
    if (localMoves.value.length && !localOver.value)
      ask({
        title: 'Start a new game?',
        description: 'Your computer game is still in progress and will be lost.',
        label: 'New game',
        run: begin,
      })
    else begin()
  }
  async function startOnlineGame(
    minutes: number,
    increment: number,
    rated: boolean,
    target = '',
  ): Promise<void> {
    searchOpen.value = false
    selectPage('online')
    if (onlinePhase.value === 'playing' || onlinePhase.value === 'seeking') {
      toast.add({
        title:
          onlinePhase.value === 'playing'
            ? 'A game is already in progress'
            : 'Already looking for an opponent',
        description: 'Finish or cancel it before starting another.',
        icon: 'i-lucide-info',
        color: 'warning',
      })
      return
    }
    onlineMinutes.value = minutes
    onlineIncrement.value = increment
    onlineRated.value = rated
    onlineChoice.value = 'random'
    onlineTarget.value = target
    await startOnline()
  }
  /** Send a friend a challenge with the form's time control (or 10+0 when that is too fast to challenge with). */
  function challengeFriend(username: string, rated = false): void {
    const [minutes, increment] = canPlayOnline(onlineMinutes.value, onlineIncrement.value, true)
      ? [onlineMinutes.value, onlineIncrement.value]
      : [10, 0]
    void startOnlineGame(minutes, increment, rated, username)
  }
  // The same keys the Puzzles and Practice pages remember their tab in (VueUse keeps every reader in sync).
  const puzzleTab = useLocalStorage('kchess:puzzle-tab', 'train')
  const practiceTab = useLocalStorage('kchess:practice-tab', 'coordinates')
  const pagesForSearch = [
    ...nav,
    { id: 'settings' as Page, label: 'Settings', icon: 'i-lucide-settings-2' },
  ]
  const searchGroups = computed(() => {
    const computerLevels: { id: EngineLevel; label: string }[] = [
      { id: 'low', label: 'Low' },
      { id: 'medium', label: 'Medium' },
      { id: 'high', label: 'High' },
    ]
    const computerItems = computerLevels.flatMap((entry) =>
      (['white', 'black'] as const).map((color) => ({
        id: `stockfish-${entry.id}-${color}`,
        label: `Play Stockfish · ${entry.label} · as ${color === 'white' ? 'White' : 'Black'}`,
        icon: 'i-lucide-cpu',
        onSelect: () => startComputerGame(entry.id, color),
      })),
    )
    const onlineItems = connectedAccounts.value.length
      ? [
          [10, 0],
          [10, 5],
          [15, 10],
          [30, 0],
        ].flatMap(([minutes, increment]) =>
          [false, true].map((rated) => ({
            id: `online-${minutes}-${increment}-${rated}`,
            label: `Find online game · ${minutes}+${increment} ${perfFor(minutes!, increment!)} · ${rated ? 'Rated' : 'Casual'}`,
            icon: 'i-lucide-globe-2',
            onSelect: () => void startOnlineGame(minutes!, increment!, rated),
          })),
        )
      : [
          {
            id: 'online-connect',
            label: 'Connect Lichess to play online',
            icon: 'i-lucide-link',
            onSelect: () => chooseSearch('settings'),
          },
        ]
    // A direct challenge allows faster controls than a public seek, so use the form's control when it qualifies.
    const [challengeMinutes, challengeIncrement] = canPlayOnline(
      onlineMinutes.value,
      onlineIncrement.value,
      true,
    )
      ? [onlineMinutes.value, onlineIncrement.value]
      : [10, 0]
    const friendItems = connectedAccounts.value.length
      ? trackedAccounts.value.flatMap((friend) =>
          [false, true].map((rated) => ({
            id: `challenge-${friend.username}-${rated}`,
            label: `Challenge @${friend.username} · ${challengeMinutes}+${challengeIncrement} · ${rated ? 'Rated' : 'Casual'}`,
            icon: 'i-lucide-swords',
            onSelect: () =>
              void startOnlineGame(challengeMinutes, challengeIncrement, rated, friend.username),
          })),
        )
      : []
    return [
      {
        id: 'pages',
        label: 'Go to',
        items: pagesForSearch.map((item) => ({
          id: item.id,
          label: item.label,
          icon: item.icon,
          onSelect: () => chooseSearch(item.id),
        })),
      },
      {
        id: 'train',
        label: 'Puzzles and practice',
        items: [
          ['train', 'Train puzzles', 'i-lucide-puzzle'],
          ['daily', 'Daily puzzle', 'i-lucide-calendar-days'],
          ['rush', 'Puzzle Storm, Streak and Rush (local)', 'i-lucide-zap'],
          ['stats', 'Puzzle stats (from Lichess)', 'i-lucide-chart-column'],
        ]
          .map(([tab, label, icon]) => ({
            id: `puzzles-${tab}`,
            label: label!,
            icon: icon!,
            onSelect: () => {
              puzzleTab.value = tab!
              chooseSearch('puzzles')
            },
          }))
          .concat(
            [
              ['coordinates', 'Practice: board coordinates', 'i-lucide-grid-3x3'],
              ['knight', 'Practice: knight paths', 'i-lucide-crown'],
              ['endgames', 'Practice: endgame drills', 'i-lucide-swords'],
            ].map(([tab, label, icon]) => ({
              id: `practice-${tab}`,
              label: label!,
              icon: icon!,
              onSelect: () => {
                practiceTab.value = tab!
                chooseSearch('practice')
              },
            })),
          ),
      },
      { id: 'play-computer', label: 'Play the computer', items: computerItems },
      { id: 'play-online', label: 'Play online', items: [...onlineItems, ...friendItems] },
      {
        id: 'actions',
        label: 'Actions',
        items: [
          {
            id: 'sync',
            label: 'Sync Lichess games',
            icon: 'i-lucide-refresh-cw',
            onSelect: () => {
              searchOpen.value = false
              void sync()
            },
          },
        ],
      },
    ]
  })
  const connectedAccounts = computed(() => data.value?.accounts.filter((a) => a.connected) ?? [])
  const trackedAccounts = computed(() => data.value?.accounts.filter((a) => !a.connected) ?? [])
  // The connected account that plays online games (remembered on this device). Falls back
  // to the first connected account when the remembered one was disconnected.
  const onlineAccountPreference = useLocalStorage('kchess:online-account', '')
  const activeOnlineAccount = computed(
    () =>
      pickConnectedAccount(data.value?.accounts ?? [], onlineAccountPreference.value)?.username ??
      '',
  )
  function setOnlineAccount(username: string): void {
    onlineAccountPreference.value = username
  }
  // Sidebar identifies only accounts the user owns (connected via OAuth), never tracked ones.
  const sidebarUserLabel = computed(() =>
    activeOnlineAccount.value ? `@${activeOnlineAccount.value}` : 'Not connected',
  )
  function defaultAccount(accounts: { username: string; connected: boolean }[]): string {
    return (accounts.find((a) => a.connected) ?? accounts[0])?.username ?? ''
  }
  const userMenuItems = computed<DropdownMenuItem[][]>(() => [
    [
      {
        label: connectedAccounts.value.length > 1 ? 'Play as' : sidebarUserLabel.value,
        icon: 'i-lucide-user',
        type: 'label' as const,
      },
      // With several accounts, the menu is where you switch which one plays online.
      ...(connectedAccounts.value.length > 1
        ? connectedAccounts.value.map((account) => ({
            label: `@${account.username}`,
            type: 'checkbox' as const,
            checked: account.username === activeOnlineAccount.value,
            onUpdateChecked: () => setOnlineAccount(account.username),
          }))
        : []),
    ],
    [
      {
        label: connectedAccounts.value.length
          ? 'Connect another account'
          : 'Connect Lichess account',
        icon: 'i-lucide-link',
        onSelect: () => void connect(),
      },
      {
        label: 'Settings',
        icon: 'i-lucide-settings-2',
        kbds: ['meta', ','],
        onSelect: () => choosePage('settings'),
      },
    ],
  ])
  function toggleSearch(): void {
    searchOpen.value = !searchOpen.value
  }
  function chooseSearch(next: Page): void {
    selectPage(next)
    searchOpen.value = false
  }
  /** True while the form differs from what is stored; auto-save clears it. */
  const settingsDirty = computed(
    () =>
      Boolean(settings.value && data.value) &&
      JSON.stringify(settings.value) !== JSON.stringify(data.value?.settings),
  )
  // Settings save themselves. Saves run one at a time, and each is a no-op
  // when nothing changed, so explicit calls (engine actions) and the debounced
  // watcher below never double-write.
  let saveChain: Promise<void> = Promise.resolve()
  function save(): Promise<void> {
    saveChain = saveChain.then(persistSettings)
    return saveChain
  }
  async function persistSettings(): Promise<void> {
    if (!settings.value || !data.value || !settingsDirty.value) return
    const sent = { ...settings.value }
    const previousEngine = data.value.settings.enginePath
    try {
      const saved = await window.kchess.saveSettings(sent)
      data.value = { ...data.value, settings: saved }
      // Keep edits made while the request was in flight; only adopt the stored
      // (normalised) copy when the form still matches what was sent.
      if (settings.value && JSON.stringify(settings.value) === JSON.stringify(sent))
        settings.value = { ...saved }
      if (saved.enginePath !== previousEngine) void refreshEngine()
    } catch (cause) {
      fail(cause)
      // Show what is actually stored rather than a change that did not stick.
      if (data.value) settings.value = { ...data.value.settings }
    }
  }
  const saveSoon = useDebounceFn(() => void save(), 400)
  watch(
    settings,
    () => {
      if (settingsDirty.value) void saveSoon()
    },
    { deep: true },
  )
  /** Write pending edits at once, e.g. when the window is closing. */
  function flushSettings(): void {
    if (settingsDirty.value) void save()
  }
  async function refreshEngine(): Promise<void> {
    engineChecking.value = true
    try {
      const result = await window.kchess.engineStatus()
      engineInfo.value = result
      engineReady.value = result.ready
    } finally {
      engineChecking.value = false
    }
  }
  /** Which Stockfish is in use, in words; empty until the first check lands. */
  const engineName = computed(() => {
    const status = engineInfo.value
    if (!status?.ready) return ''
    if (status.bundled) return 'Bundled Stockfish'
    return status.path === status.managed.path
      ? `Downloaded Stockfish${status.managed.version ? ` ${status.managed.version}` : ''}`
      : 'Your Stockfish'
  })
  async function recheckEngine(): Promise<void> {
    await refreshEngine().catch(() => {
      engineReady.value = false
    })
  }
  async function chooseEngine(): Promise<void> {
    const path = await window.kchess.chooseEngine()
    if (path && settings.value) {
      settings.value.enginePath = path
      await save()
    }
  }
  async function useBundledEngine(): Promise<void> {
    if (!settings.value) return
    settings.value.enginePath = ''
    await save()
  }
  /**
   * Check for the latest native Stockfish and download it if needed. A first
   * download becomes the engine in use; updating an existing copy leaves the
   * current choice alone.
   */
  async function installEngine(): Promise<void> {
    const hadDownload = engineInfo.value?.managed.installed ?? false
    const result = await run(() => window.kchess.installEngine())
    if (!result || !settings.value) return
    if (!result.updated) {
      info(`You already have the latest Stockfish (${result.version}).`)
      await refreshEngine()
      return
    }
    if (hadDownload) {
      await refreshEngine()
      info(`Stockfish updated to ${result.version}.`)
      return
    }
    settings.value.enginePath = result.path
    await save()
    info(`Stockfish ${result.version} downloaded.`)
  }
  async function useDownloadedEngine(): Promise<void> {
    const path = engineInfo.value?.managed.path
    if (!path || !settings.value) return
    settings.value.enginePath = path
    await save()
  }
  /** Delete the downloaded engine; if it was in use, fall back to the bundled one. */
  async function deleteEngine(): Promise<void> {
    const managedPath = engineInfo.value?.managed.path
    const deleted = await run(async () => {
      await window.kchess.deleteEngine()
      return true
    })
    if (!deleted) return
    if (settings.value && settings.value.enginePath === managedPath) {
      settings.value.enginePath = ''
      await save()
    } else await refreshEngine()
    info('Downloaded Stockfish deleted.')
  }
  async function addAccount(): Promise<void> {
    const result = await run(() => window.kchess.addAccount(usernameInput.value))
    if (result) {
      data.value = result
      usernameInput.value = ''
      selectedAccount.value ||= defaultAccount(result.accounts)
      await sync(result.accounts.at(-1)?.username)
    }
  }
  /** Free the space a friend's synced games and profile take, keeping them as a friend. */
  async function clearSyncedData(username: string): Promise<void> {
    const result = await run(() => window.kchess.clearAccountData(username))
    if (!result) return
    data.value = result
    if (reviewGame.value?.account.toLowerCase() === username.toLowerCase()) reviewGame.value = null
    void usage.refresh()
    info(`Cleared @${username}'s synced data. They are still your friend; sync to download again.`)
  }
  /** Add followed players as friends. Their games are not downloaded; each friend is synced on request. */
  async function importFriends(usernames: string[]): Promise<boolean> {
    // A plain copy: Electron's IPC cannot clone Vue's reactive arrays ("An object could not be cloned").
    const names = [...usernames]
    const result = await run(() => window.kchess.addFriends(names))
    if (!result) return false
    data.value = result
    void usage.refresh()
    info(
      `Added ${usernames.length} ${usernames.length === 1 ? 'friend' : 'friends'}. Their games aren't downloaded until you sync them.`,
    )
    return true
  }
  async function removeAccount(username: string): Promise<void> {
    const result = await run(() => window.kchess.removeAccount(username))
    if (result) {
      data.value = result
      selectedAccount.value = defaultAccount(result.accounts)
      if (historyAccount.value.toLowerCase() === username.toLowerCase())
        historyAccount.value = 'all'
      void usage.refresh()
      // The removal is stored on this device, so the account stays gone until it is added again.
      info(`@${username} removed. Their stored games were deleted from this device.`)
    }
  }
  async function connect(): Promise<void> {
    const knownBefore = new Set(
      connectedAccounts.value.map((account) => account.username.toLowerCase()),
    )
    const result = await run(() => window.kchess.connectLichess())
    if (!result) return
    data.value = result.data
    selectedAccount.value = result.username
    if (knownBefore.has(result.username.toLowerCase())) {
      // Lichess authorises whoever is signed in to the browser, so this is the same account again.
      info(
        `@${result.username} was already connected, so its login was refreshed. To add another account, sign in to it on lichess.org first.`,
      )
      return
    }
    setOnlineAccount(result.username)
    info(`Connected @${result.username}.`)
    await sync(result.username)
  }
  async function sync(username?: string): Promise<void> {
    const before = (await usage.refresh())?.accounts
    const result = await run(() => window.kchess.syncGames(username))
    if (result) {
      data.value = result
      // Say what the sync cost, so downloads are never a surprise.
      const after = (await usage.refresh())?.accounts
      const downloaded = Object.entries(after ?? {}).reduce(
        (sum, [account, entry]) =>
          sum + entry.total.bytesIn - (before?.[account]?.total.bytesIn ?? 0),
        0,
      )
      info(
        downloaded > 0
          ? `Lichess games synced · ${formatBytes(downloaded)} downloaded`
          : 'Lichess games synced.',
      )
      void loadProfile()
    }
  }
  function applyRatingHistory(history: LichessRatingHistory): void {
    ratingHistories.value = history
  }
  async function loadProfile(): Promise<void> {
    const account = selectedAccount.value
    if (!account) {
      profile.value = null
      ratingHistories.value = []
      return
    }
    try {
      // Paint what was saved last time straight away, then refresh from Lichess.
      const saved = await window.kchess.cachedProfile(account)
      if (account !== selectedAccount.value) return
      profile.value = saved.profile
      if (saved.ratingHistory) applyRatingHistory(saved.ratingHistory)
      else ratingHistories.value = []
      const [p, history] = await Promise.all([
        window.kchess.profile(account),
        window.kchess.ratingHistory(account),
      ])
      if (account !== selectedAccount.value) return
      profile.value = p
      applyRatingHistory(history)
    } catch (cause) {
      fail(cause)
    }
  }
  watch(selectedAccount, () => {
    void loadProfile()
  })
  watch([historyResult, historyRated, historyAccount, historyPageSize], () => {
    historyPage.value = 0
  })
  // Sound and appearance follow the form as it is edited; saving happens in the background.
  const systemDark = window.matchMedia('(prefers-color-scheme: dark)')
  /** Themes the user dropped in the themes folder. */
  const customThemes = ref<AppTheme[]>([])
  const themeProblems = ref<string[]>([])
  const themesDir = ref('')
  const themeList = computed(() => allThemes(customThemes.value))
  /** Whether the app is showing its dark variant (the appearance setting, or the system's). */
  const darkMode = ref(false)
  function applySettings(value: Settings): void {
    configure({ enabled: value.soundEnabled, volume: value.soundVolume })
    const dark =
      value.appearance === 'dark' || (value.appearance === 'system' && systemDark.matches)
    darkMode.value = dark
    document.documentElement.classList.toggle('dark', dark)
    const chosen = dark ? value.darkTheme : value.lightTheme
    applyTheme(document.documentElement, findTheme(chosen, customThemes.value), dark)
  }
  async function reloadThemes(): Promise<void> {
    try {
      const report = await window.kchess.loadThemes()
      customThemes.value = report.themes
      themeProblems.value = report.problems
      themesDir.value = report.dir
    } catch (cause) {
      themeProblems.value = [cause instanceof Error ? cause.message : 'Could not read the themes.']
    }
  }
  async function openThemesFolder(): Promise<void> {
    await window.kchess.openThemesFolder().catch(fail)
  }
  function applyCurrentSettings(): void {
    if (settings.value) applySettings(settings.value)
  }
  watch([settings, customThemes], applyCurrentSettings, { deep: true, immediate: true })

  function makeMove(uci: string): void {
    if (localPly.value !== localMoves.value.length || localOver.value || thinking.value) return
    const san = playUci(positionAfter(localMoves.value), uci)
    if (!san) return
    localMoves.value = [...localMoves.value, uci]
    localPly.value = localMoves.value.length
    playMoveSound(san)
    void computerTurn()
  }
  async function computerTurn(): Promise<void> {
    if (localGame.value.turn === userColor.value || localOver.value || !engineReady.value) return
    const epoch = gameEpoch.value,
      moveCount = localMoves.value.length
    thinking.value = true
    try {
      const move = await window.kchess.bestMove([...localMoves.value], level.value)
      if (epoch !== gameEpoch.value || moveCount !== localMoves.value.length) return
      const san = playUci(positionAfter(localMoves.value), move)
      if (!san) return
      localMoves.value = [...localMoves.value, move]
      localPly.value = localMoves.value.length
      playMoveSound(san)
      notifyDesktop('computerMove', 'Stockfish moved', `${san}. Your move.`)
    } catch (cause) {
      // A new game or takeback cancels the search; that is not an error.
      if (epoch === gameEpoch.value) {
        fail(cause)
        // The engine may have vanished since the last check; make the indicator tell the truth.
        void recheckEngine()
      }
    } finally {
      if (epoch === gameEpoch.value) thinking.value = false
    }
  }
  function newGame(): void {
    gameEpoch.value++
    thinking.value = false
    localMoves.value = []
    localPly.value = 0
    flipped.value = userColor.value === 'black'
    void computerTurn()
  }
  function takeback(): void {
    if (!localMoves.value.length) return
    gameEpoch.value++
    thinking.value = false
    localMoves.value = takebackMoves(localMoves.value, userColor.value)
    localPly.value = localMoves.value.length
    // Playing black and undoing back to the start hands the move to the computer.
    void computerTurn()
  }
  function localStatus(): string {
    if (localGame.value.isCheckmate()) return 'Checkmate'
    if (localGame.value.isEnd()) return 'Draw'
    if (localDraw.value) return `Draw · ${localDraw.value}`
    if (localGame.value.isCheck()) return 'Check'
    return thinking.value ? 'Computer is thinking…' : `Your move · ${statusText(localGame.value)}`
  }
  async function openReview(game: LichessGame): Promise<void> {
    // List rows carry no PGN; it is read locally on demand.
    const pgn = game.pgn ?? (await window.kchess.gamePgn(game.account, game.id).catch(() => null))
    reviewGame.value = pgn ? { ...game, pgn } : game
    selectPage('history')
  }

  async function startOnline(): Promise<void> {
    const result = await run(() =>
      window.kchess.startOnline({
        minutes: onlineMinutes.value,
        increment: onlineIncrement.value,
        color: onlineChoice.value,
        rated: onlineRated.value,
        target: onlineTarget.value || undefined,
        account: activeOnlineAccount.value || undefined,
      }),
    )
    if (result) {
      onlinePhase.value = 'seeking'
      onlineStatus.value = result.url
        ? 'Challenge sent. Waiting for acceptance…'
        : 'Looking for an opponent…'
    }
  }
  async function stopOnline(): Promise<void> {
    await window.kchess.cancelOnline()
    onlinePhase.value = 'idle'
    onlineStatus.value = ''
    onlineViewPly.value = null
  }
  function readOnlineEvent(event: OnlineEvent): void {
    if (event.type === 'gameStart') {
      onlineId.value = event.game.gameId
      onlineColor.value = event.game.color ?? 'white'
      onlineOpponent.value = event.game.opponent?.username ?? 'Opponent'
      onlineMoves.value = []
      onlineViewPly.value = null
      onlineFlipped.value = false
      onlineGameRated.value = event.game.rated ?? false
      opponentId.value = event.game.opponent?.id ?? ''
      opponentGone.value = false
      presence.value = null
      onlineInitial.value = 0
      onlinePhase.value = 'playing'
      onlineStatus.value = 'Game in progress'
      notifyDesktop(
        'gameEvents',
        'Game started',
        `You are playing ${onlineOpponent.value} as ${onlineColor.value}.`,
      )
      return
    }
    if (event.type === 'gameFinish') {
      const wasPlaying = onlinePhase.value === 'playing'
      const reason = event.game.status?.name ?? 'finished'
      onlinePhase.value = 'finished'
      onlineStatus.value = `Game ended: ${reason}`
      if (wasPlaying) {
        const winner = event.game.winner
        const outcome =
          reason === 'aborted'
            ? 'Game aborted'
            : !winner
              ? 'Draw'
              : winner === (event.game.color ?? onlineColor.value)
                ? 'You won'
                : 'You lost'
        notifyDesktop('gameEvents', outcome, `Against ${onlineOpponent.value} · ${reason}.`)
      }
      return
    }
    if (event.type === 'opponentGone') {
      if (onlinePhase.value === 'playing') {
        opponentGone.value = event.gone
        onlineStatus.value = event.gone
          ? event.claimWinInSeconds
            ? `Opponent disconnected… you can claim the win in ${event.claimWinInSeconds}s`
            : 'Opponent disconnected…'
          : 'Game in progress'
      }
      return
    }
    // Challenges and chat lines carry no game state; treating them as one would wipe the board.
    if (event.type !== 'gameFull' && event.type !== 'gameState') return
    const state = event.type === 'gameFull' ? event.state : event
    const previousCount = onlineMoves.value.length
    if (event.type === 'gameFull') {
      onlineId.value = event.id
      onlineInitial.value = (event.clock?.initial ?? 0) / 1000
      onlineGameRated.value = event.rated
      const ours = activeOnlineAccount.value.toLowerCase()
      // Match by account; if neither side does, keep the colour `gameStart` announced.
      if (event.white.id?.toLowerCase() === ours) onlineColor.value = 'white'
      else if (event.black.id?.toLowerCase() === ours) onlineColor.value = 'black'
      const opponent = onlineColor.value === 'white' ? event.black : event.white
      onlineOpponent.value = opponent.name ?? 'Opponent'
      opponentId.value = opponent.id ?? ''
    }
    onlineMoves.value = String(state.moves ?? '')
      .trim()
      .split(/\s+/)
      .filter(Boolean)
    if (previousCount && onlineMoves.value.length > previousCount) {
      const san = sanHistory(onlineMoves.value).at(-1)
      playMoveSound(san)
      // Even plies are white's moves; a move by the other colour is the opponent's.
      const mover = (onlineMoves.value.length - 1) % 2 === 0 ? 'white' : 'black'
      if (mover !== onlineColor.value)
        notifyDesktop('opponentMove', `${onlineOpponent.value} moved`, `${san}. Your move.`)
    }
    const running = isGameInProgress(state.status)
    clock.set({
      white: Number(state.wtime ?? 0),
      black: Number(state.btime ?? 0),
      ticking: running
        ? onlineMoves.value.length
          ? turnColor(positionAfter(onlineMoves.value))
          : 'white'
        : undefined,
      initialSeconds: onlineInitial.value || undefined,
    })
    if (running) {
      onlinePhase.value = 'playing'
      onlineStatus.value = 'Game in progress'
    } else {
      onlinePhase.value = 'finished'
      onlineStatus.value = `Game ended: ${state.status}`
    }
  }
  async function onlineMove(uci: string): Promise<void> {
    if (!onlineId.value || onlinePhase.value !== 'playing') return
    const sent = performance.now()
    try {
      await window.kchess.playOnline(onlineId.value, uci)
      moveAckMs.value = Math.round(performance.now() - sent)
    } catch (cause) {
      fail(cause)
    }
  }
  async function onlineAction(
    action: 'resign' | 'abort' | 'takeback' | 'declineTakeback',
  ): Promise<void> {
    if (!onlineId.value) return
    await run(() => window.kchess.onlineAction(onlineId.value, action))
  }
  function clockText(color: 'white' | 'black'): string {
    void now.value
    return formatClock(clock.remaining(color))
  }
  const now = ref(performance.now())
  function keydown(event: KeyboardEvent): void {
    if ((event.metaKey || event.ctrlKey) && event.key === ',') {
      event.preventDefault()
      selectPage('settings')
    }
    if (
      (event.metaKey || event.ctrlKey) &&
      (event.key.toLowerCase() === 'k' || event.key.toLowerCase() === 'f')
    ) {
      event.preventDefault()
      toggleSearch()
      return
    }
    // The palette is a modal that owns the keyboard while it is open.
    if (searchOpen.value) return
    if (
      event.target instanceof HTMLInputElement ||
      event.target instanceof HTMLSelectElement ||
      event.target instanceof HTMLTextAreaElement
    )
      return
    if (page.value === 'computer') {
      const ply = navigatePly(event.key, localPly.value, localMoves.value.length)
      if (ply !== undefined) {
        localPly.value = ply
        event.preventDefault()
      }
    } else if (page.value === 'online' && ['playing', 'finished'].includes(onlinePhase.value)) {
      const ply = navigatePly(event.key, onlinePly.value, onlineMoves.value.length)
      if (ply !== undefined) {
        viewOnlinePly(ply)
        event.preventDefault()
      }
    }
  }
  async function init(): Promise<void> {
    if (initialized) return
    initialized = true
    try {
      data.value = await window.kchess.loadData()
      settings.value = { ...data.value.settings }
      selectedAccount.value = defaultAccount(data.value.accounts)
      // The engine check is not needed to show the app; it flips `engineReady` when it lands.
      void refreshEngine().catch(() => undefined)
      void reloadThemes()
      error.value = ''
    } catch (cause) {
      initialized = false
      fail(cause)
      return
    }
    systemDark.addEventListener('change', applyCurrentSettings)
    window.addEventListener('beforeunload', flushSettings)
    offOnline = window.kchess.onOnlineEvent(readOnlineEvent)
    offOnlineError = window.kchess.onOnlineError(fail)
    offNotification = window.kchess.onNotification(({ title, body }) =>
      toast.add({ title, description: body, icon: 'i-lucide-bell', duration: 8000 }),
    )
    void window.kchess
      .resumeOnline()
      .then((resumed) => {
        if (resumed) {
          // The game may belong to any connected account; make that the active one.
          setOnlineAccount(resumed.account)
          onlineId.value = resumed.id
          onlinePhase.value = 'playing'
        }
      })
      .catch(() => undefined)
    ticker.resume()
    window.addEventListener('keydown', keydown)
    void loadProfile()
  }
  function dispose(): void {
    offOnline?.()
    offOnlineError?.()
    offOnline = undefined
    offOnlineError = undefined
    offNotification?.()
    offNotification = undefined
    ticker.pause()
    presenceTicker.pause()
    window.removeEventListener('keydown', keydown)
    systemDark.removeEventListener('change', applyCurrentSettings)
    window.removeEventListener('beforeunload', flushSettings)
    initialized = false
  }
  const ready = computed(() => data.value !== null && settings.value !== null)

  return {
    ready,
    sidebarOpen,
    navItems,
    userMenuItems,
    sidebarUserLabel,
    activeOnlineAccount,
    setOnlineAccount,
    connectedAccounts,
    trackedAccounts,
    busy,
    error,
    message,
    searchOpen,
    searchGroups,
    confirmation,
    settingsSection,
    startOnlineGame,
    challengeFriend,
    importFriends,
    clearSyncedData,
    toggleSearch,
    chooseSearch,
    selectedAccount,
    profile,
    chartMode,
    chartRange,
    games,
    recentGames,
    counts,
    chartSeries,
    chartValues,
    chartFromGames,
    sync,
    selectPage,
    openReview,
    localMoves,
    localPly,
    level,
    userColor,
    flipped,
    thinking,
    engineReady,
    engineChecking,
    engineName,
    recheckEngine,
    engineInfo,
    localDisplay,
    localHistory,
    localLast,
    localDests,
    localTurn,
    localCheck,
    localInteractive,
    localCanPlay,
    localResult,
    newGame,
    takeback,
    localStatus,
    makeMove,
    fen,
    onlinePhase,
    onlineMinutes,
    onlineIncrement,
    onlineChoice,
    onlineRated,
    onlineGameRated,
    onlineTarget,
    startOnline,
    stopOnline,
    onlineStatus,
    onlineOpponent,
    onlineColor,
    onlinePosition,
    onlineHistory,
    onlineLast,
    onlineDests,
    onlineTurn,
    onlineCheck,
    onlineInteractive,
    onlineCanPlay,
    onlinePly,
    onlineDisplay,
    onlineDisplayTurn,
    onlineOrientation,
    onlineFlipped,
    onlinePresence,
    pingStats,
    moveAckMs,
    presenceIntervalSeconds: PRESENCE_INTERVAL_MS / 1000,
    presence,
    viewOnlinePly,
    clockText,
    onlineId,
    onlineAction,
    onlineMove,
    reviewGame,
    date,
    reviewPgn,
    historyAccount,
    historyResult,
    historyRated,
    historyPage,
    historyPageSize,
    filteredGames,
    visibleGames,
    historyPageCount,
    save,
    boardThemes,
    themeList,
    themeProblems,
    themesDir,
    darkMode,
    reloadThemes,
    openThemesFolder,
    useBundledEngine,
    installEngine,
    useDownloadedEngine,
    deleteEngine,
    chooseEngine,
    usernameInput,
    addAccount,
    connect,
    removeAccount,
    data: computed(() => data.value!),
    settings: computed(() => settings.value!),
    init,
    dispose,
  }
})

export type KChessStore = ReturnType<typeof useKChessStore>
