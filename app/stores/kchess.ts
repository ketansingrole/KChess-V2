import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import { useDebounceFn, useLocalStorage, useMediaQuery } from '@vueuse/core'
import type { DropdownMenuItem, NavigationMenuItem } from '@nuxt/ui'
import { pickConnectedAccount } from '../../src/shared/accounts'
import {
  DEFAULT_ENGINE_LEVELS,
  engineLevelLabel,
  nearestEngineLevel,
} from '../../src/shared/engineLevels'
import { canPlayOnline, perfFor } from '../../src/shared/timeControl'
import { boardThemes } from '../utils/boards'
import { allThemes, applyTheme, findTheme } from '../utils/themes'
import { fen, navigatePly } from '../utils/chess'
import { configure } from '../utils/sound'
import { formatGameDate } from '../utils/games'
import { useOnlineGame } from './kchess/onlineGame'
import { useComputerGame } from './kchess/computerGame'
import { useGameHistory } from './kchess/gameHistory'
import { formatBytes } from '../utils/format'
import { useUsageStore } from './usage'
import type {
  AppData,
  EngineLevel,
  EngineStatus,
  LichessRatingHistory,
  LichessUser,
  AppTheme,
  NotificationKind,
  Settings,
} from '../../src/shared/types'

export type Page =
  | 'dashboard'
  | 'online'
  | 'computer'
  | 'analysis'
  | 'editor'
  | 'puzzles'
  | 'practice'
  | 'history'
  | 'friends'
  | 'settings'

/** Sections of the Settings page; while it is open they replace the sidebar's pages. */
export const SETTINGS_SECTIONS = [
  { id: 'appearance', label: 'Appearance', icon: 'i-lucide-palette' },
  { id: 'themes', label: 'App theme', icon: 'i-lucide-swatch-book' },
  { id: 'gameplay', label: 'Gameplay', icon: 'i-lucide-gamepad-2' },
  { id: 'sound', label: 'Sound', icon: 'i-lucide-volume-2' },
  { id: 'notifications', label: 'Notifications', icon: 'i-lucide-bell' },
  { id: 'voice', label: 'Voice input', icon: 'i-lucide-mic' },
  { id: 'engine', label: 'Chess engine', icon: 'i-lucide-cpu' },
  { id: 'accounts', label: 'My accounts', icon: 'i-lucide-user-round' },
  { id: 'data', label: 'Data & storage', icon: 'i-lucide-database' },
  { id: 'updates', label: 'Updates', icon: 'i-lucide-download' },
] as const
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]['id']

export const useKChessStore = defineStore('kchess', () => {
  /** Pages in the sidebar. Settings is reached from the account menu instead. */
  const nav: { id: Page; label: string; icon: string }[] = [
    { id: 'dashboard', label: 'Dashboard', icon: 'i-lucide-layout-dashboard' },
    { id: 'online', label: 'Play Online', icon: 'i-lucide-globe-2' },
    { id: 'computer', label: 'Play with Computer', icon: 'i-lucide-monitor' },
    { id: 'analysis', label: 'Analysis board', icon: 'i-lucide-microscope' },
    { id: 'editor', label: 'Board editor', icon: 'i-lucide-pencil-ruler' },
    { id: 'puzzles', label: 'Puzzles', icon: 'i-lucide-puzzle' },
    { id: 'practice', label: 'Practice', icon: 'i-lucide-graduation-cap' },
    { id: 'history', label: 'History', icon: 'i-lucide-history' },
    { id: 'friends', label: 'Friends', icon: 'i-lucide-users' },
  ]
  const route = useRoute()
  const page = computed<Page>(() => {
    const section = route.path.slice(1)
    return [
      'online',
      'computer',
      'analysis',
      'editor',
      'puzzles',
      'practice',
      'history',
      'friends',
      'settings',
    ].includes(section)
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
  const {
    historyResult,
    historyRated,
    historyAccount,
    historyPage,
    historyPageSize,
    reviewGame,
    reviewPgn,
    games,
    filteredGames,
    historyPageCount,
    visibleGames,
    recentGames,
    counts,
    chartSeries,
    chartValues,
    chartFromGames,
    openReview,
  } = useGameHistory({
    data,
    selectedAccount,
    ratingHistories,
    chartMode,
    chartRange,
    selectPage,
  })
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
  const {
    localMoves,
    localGameEpoch,
    localPly,
    level,
    userColor,
    thinking,
    localOver,
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
    resign,
    localStatus,
    makeMove,
  } = useComputerGame({
    engineReady,
    fail,
    recheckEngine,
    notifyDesktop,
  })
  // A level turned off in Settings gives way to the closest one still offered.
  watch(
    () => settings.value?.engineLevels,
    (enabled) => {
      if (enabled) level.value = nearestEngineLevel(level.value, enabled)
    },
    { immediate: true },
  )
  const {
    onlinePhase,
    onlineId,
    onlineMoves,
    onlineColor,
    onlineOpponent,
    onlineStatus,
    onlineMinutes,
    onlineIncrement,
    onlineTarget,
    onlineChoice,
    onlineRated,
    onlineGameRated,
    onlineFlipped,
    presence,
    moveAckMs,
    ticker,
    presenceTicker,
    onlinePosition,
    onlineHistory,
    onlinePly,
    onlineDisplay,
    onlineLast,
    onlineDests,
    onlineTurn,
    onlineDisplayTurn,
    onlineCheck,
    onlineCanPlay,
    onlineInteractive,
    viewOnlinePly,
    onlineOrientation,
    onlinePresence,
    pingStats,
    PRESENCE_INTERVAL_MS,
    startOnline,
    stopOnline,
    readOnlineEvent,
    onlineMove,
    onlineAction,
    clockText,
  } = useOnlineGame({
    activeAccount: () => activeOnlineAccount.value,
    run,
    fail,
    notifyDesktop,
  })
  /** Ask the main process for a desktop notification; it applies the Settings and window-state rules. */
  function notifyDesktop(kind: NotificationKind, title: string, body: string): void {
    void window.kchess.notify({ kind, title, body }).catch(() => undefined)
  }
  let offOnline: (() => void) | undefined
  let offOnlineError: (() => void) | undefined
  let offNotification: (() => void) | undefined
  let initialized = false
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
    document.querySelector('.main-content')?.scrollTo({ top: 0 })
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
    const computerLevels = (settings.value?.engineLevels ?? DEFAULT_ENGINE_LEVELS).map((id) => ({
      id,
      label: engineLevelLabel(id),
    }))
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
    // A reactive array cannot cross IPC (structured clone): send a plain copy of the list.
    const sent = { ...settings.value, engineLevels: [...settings.value.engineLevels] }
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
    localGameEpoch,
    localPly,
    level,
    userColor,
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
    resign,
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
