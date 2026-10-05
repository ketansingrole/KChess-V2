import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import { useLocalStorage, useMediaQuery } from '@vueuse/core'
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
import { useEngineSettings } from './kchess/engineSettings'
import { useSettingsPersistence } from './kchess/settingsPersistence'
import { useAccountProfile } from './kchess/accountProfile'
import { formatBytes } from '../utils/format'
import { useUsageStore } from './usage'
import type { GameSetup } from '../../src/shared/variant'
import type {
  AppData,
  EngineLevel,
  AppTheme,
  NotificationKind,
  Settings,
} from '../../src/shared/types'

export type Page =
  | 'dashboard'
  | 'online'
  | 'tournaments'
  | 'watch'
  | 'local'
  | 'computer'
  | 'analysis'
  | 'studies'
  | 'editor'
  | 'puzzles'
  | 'practice'
  | 'history'
  | 'insights'
  | 'friends'
  | 'players'
  | 'settings'

/** Sections of the Settings page; while it is open they replace the sidebar's pages. */
export const SETTINGS_SECTIONS = [
  { id: 'appearance', label: 'Appearance', icon: 'i-lucide-palette' },
  { id: 'themes', label: 'App theme', icon: 'i-lucide-swatch-book' },
  { id: 'gameplay', label: 'Gameplay', icon: 'i-lucide-gamepad-2' },
  { id: 'online', label: 'Online play', icon: 'i-lucide-globe-2' },
  { id: 'analysis', label: 'Analysis', icon: 'i-lucide-microscope' },
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
    { id: 'tournaments', label: 'Tournaments', icon: 'i-lucide-trophy' },
    { id: 'computer', label: 'Play with Computer', icon: 'i-lucide-monitor' },
    { id: 'local', label: 'Over the board', icon: 'i-lucide-users-round' },
    { id: 'watch', label: 'Watch', icon: 'i-lucide-tv' },
    { id: 'analysis', label: 'Analysis board', icon: 'i-lucide-microscope' },
    { id: 'studies', label: 'Studies', icon: 'i-lucide-library-big' },
    { id: 'editor', label: 'Board editor', icon: 'i-lucide-pencil-ruler' },
    { id: 'puzzles', label: 'Puzzles', icon: 'i-lucide-puzzle' },
    { id: 'practice', label: 'Practice', icon: 'i-lucide-graduation-cap' },
    { id: 'history', label: 'History', icon: 'i-lucide-history' },
    { id: 'insights', label: 'Insights', icon: 'i-lucide-chart-pie' },
    { id: 'friends', label: 'Friends', icon: 'i-lucide-users' },
    { id: 'players', label: 'Players', icon: 'i-lucide-user-search' },
  ]
  const route = useRoute()
  const page = computed<Page>(() => {
    const section = route.path.slice(1)
    return [
      'online',
      'tournaments',
      'watch',
      'local',
      'computer',
      'analysis',
      'studies',
      'editor',
      'puzzles',
      'practice',
      'history',
      'insights',
      'friends',
      'players',
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
  const activeOperations = ref(0)
  const busy = computed(() => activeOperations.value > 0)
  const message = ref('')
  const error = ref('')
  const usernameInput = ref('')
  const selectedAccount = ref('')
  const { profile, ratingHistories, loadProfile } = useAccountProfile(selectedAccount, fail)
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
    gameCount,
    selectedGameCount,
    libraryOverview,
    historyTotal,
    historyLoading,
    historyError,
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
  const {
    engineReady,
    engineChecking,
    engineInfo,
    engineName,
    refreshEngine,
    recheckEngine,
    chooseEngine,
    useBundledEngine,
    installEngine,
    useDownloadedEngine,
    deleteEngine,
  } = useEngineSettings({ settings, run, info, save })
  const persistence = useSettingsPersistence({ settings, data, fail, refreshEngine })
  function save(): Promise<void> {
    return persistence.save()
  }
  const { flushSettings } = persistence
  /** A yes/no question raised outside any page (e.g. from the search palette). */
  const confirmation = ref<{
    title: string
    description: string
    label: string
    run: () => void
  } | null>(null)
  const onlineGame = useOnlineGame({
    activeAccount: () => activeOnlineAccount.value,
    run,
    fail,
    info,
    notifyDesktop,
    chatEnabled: () => settings.value?.onlineChat ?? true,
  })
  const {
    localMoves,
    localSaveError,
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
    localGame,
    localSetup,
    localClock,
    computerClockText,
    newGame,
    takeback,
    resign,
    localStatus,
    makeMove,
  } = useComputerGame({
    engineReady,
    assistanceAllowed: () =>
      !['playing', 'seeking', 'disconnected'].includes(onlineGame.onlinePhase.value),
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
    onlineStatus,
    onlineAccount,
    readOnlineState,
    onlineId,
    onlineMoves,
    onlineMinutes,
    onlineIncrement,
    onlineTarget,
    onlineChoice,
    onlineRated,
    ticker,
    presenceTicker,
    onlinePly,
    viewOnlinePly,
    PRESENCE_INTERVAL_MS,
    startOnline,
    readOnlineEvent,
  } = onlineGame
  /** A challenge was accepted: the game about to open belongs to this account. */
  function prepareForGame(account: string, id: string): void {
    onlineAccount.value = account
    onlineId.value = id
    onlineGame.onlineStatus.value = 'Starting the game…'
  }
  /** Ask the main process for a desktop notification; it applies the Settings and window-state rules. */
  function notifyDesktop(kind: NotificationKind, title: string, body: string): void {
    void window.kchess.notify({ kind, title, body }).catch(() => undefined)
  }
  let offOnline: (() => void) | undefined
  let offOnlineState: (() => void) | undefined
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
    activeOperations.value++
    error.value = ''
    try {
      return await action()
    } catch (cause) {
      fail(cause)
      return undefined
    } finally {
      activeOperations.value--
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
  /** Play the computer from a position (the editor's, the analysis board's) or a variant start. */
  function startComputerFrom(setup: GameSetup, color?: 'white' | 'black'): void {
    const begin = (): void => {
      if (color) userColor.value = color
      newGame(setup)
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

  /** Flip a yes/no setting from a quick toggle; it saves itself like the Settings page. */
  function toggleSetting(key: 'zenMode' | 'blindfold' | 'cloudEval' | 'showOpeningName'): void {
    if (settings.value) settings.value = { ...settings.value, [key]: !settings.value[key] }
  }
  /** Zen mode applies on the pages where games are played. */
  const zenActive = computed(
    () => Boolean(settings.value?.zenMode) && ['online', 'computer', 'local'].includes(page.value),
  )
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
    // Z toggles zen mode on game pages, as on Lichess.
    if (
      event.key === 'z' &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.altKey &&
      ['online', 'computer', 'local'].includes(page.value)
    ) {
      event.preventDefault()
      toggleSetting('zenMode')
      return
    }
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
    onlineGame.checkingOnline()
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
    offOnline = window.kchess.onOnlineEvent((event) => {
      readOnlineEvent(event)
      // A pairing or an accepted challenge takes you to the board, as Lichess does.
      if (
        event.type === 'gameStart' &&
        event.game.speed !== 'correspondence' &&
        page.value !== 'online'
      )
        selectPage('online')
    })
    offOnlineError = window.kchess.onOnlineError(fail)
    offOnlineState = window.kchess.onOnlineState(readOnlineState)
    offNotification = window.kchess.onNotification(({ title, body }) =>
      toast.add({ title, description: body, icon: 'i-lucide-bell', duration: 8000 }),
    )
    void window.kchess
      .resumeOnline()
      .then((resumed) => {
        if (resumed) {
          // The game may belong to any connected account; make that the active one.
          setOnlineAccount(resumed.account)
          onlineAccount.value = resumed.account
          onlineId.value = resumed.id
          if (onlinePhase.value !== 'playing') onlinePhase.value = 'disconnected'
        } else {
          onlinePhase.value = 'idle'
        }
      })
      .catch((cause) => {
        if (onlineGame.onlineConnection.value?.phase === 'checking')
          onlineGame.onlineConnection.value = {
            ...onlineGame.onlineConnection.value,
            phase: 'disconnected',
          }
        onlineStatus.value =
          cause instanceof Error ? cause.message : 'Reconnect to verify your game status.'
      })
    ticker.resume()
    window.addEventListener('keydown', keydown)
    void loadProfile()
  }
  function dispose(): void {
    offOnline?.()
    offOnlineError?.()
    offOnlineState?.()
    offOnlineState = undefined
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
    ...onlineGame,
    prepareForGame,
    toggleSetting,
    zenActive,
    page,
    runAction: run,
    notifyInfo: info,
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
    gameCount,
    selectedGameCount,
    libraryOverview,
    recentGames,
    counts,
    chartSeries,
    chartValues,
    chartFromGames,
    sync,
    selectPage,
    openReview,
    localMoves,
    localSaveError,
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
    localGame,
    localSetup,
    localClock,
    computerClockText,
    startComputerFrom,
    newGame,
    takeback,
    resign,
    localStatus,
    makeMove,
    fen,
    presenceIntervalSeconds: PRESENCE_INTERVAL_MS / 1000,
    reviewGame,
    date,
    reviewPgn,
    historyAccount,
    historyResult,
    historyRated,
    historyPage,
    historyPageSize,
    historyTotal,
    historyLoading,
    historyError,
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
