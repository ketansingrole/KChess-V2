import { computed, type Ref } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import type { CommandPaletteGroup, CommandPaletteItem } from '@nuxt/ui'
import { DEFAULT_ENGINE_LEVELS, engineLevelInfo } from '../../src/shared/engineLevels'
import { canPlayOnline, perfFor } from '../../src/shared/timeControl'
import { APPEARANCES, COORDINATE_MODES } from '../../src/shared/types'
import { SETTINGS_SECTIONS, useKChessStore, type Page } from '../stores/kchess'
import type { PuzzleTab } from '../stores/puzzles'
import { useLocalGameStore } from '../stores/local'
import { useAnalysisStore } from '../stores/analysis'
import {
  escapeHtml,
  highlightRanges,
  parseQuickPick,
  pushRecent,
  rankQuickPick,
  type QuickPickMode,
} from '../utils/quickPick'

/** One entry the palette can run. */
export interface QuickCommand {
  id: string
  label: string
  icon: string
  /** Dimmed text after the label (a level's strength, where a setting lives). */
  detail?: string
  /** Right-aligned state: "On", "Current", "In progress". */
  hint?: string
  /** Extra words that should find this entry but are not shown. */
  keywords?: string
  kbds?: string[]
  run: () => void
  /** Switch the palette's mode instead of closing it. */
  prefix?: string
}

export interface QuickPickItem extends CommandPaletteItem {
  hint?: string
}

const PRACTICE_TABS = [
  ['coordinates', 'Board coordinates', 'i-lucide-grid-3x3', 'squares names'],
  ['colors', 'Square colours', 'i-lucide-contrast', 'light dark'],
  ['knight', 'Knight paths', 'i-lucide-crown', 'knight tour'],
  ['endgames', 'Endgame drills', 'i-lucide-swords', 'mate technique'],
  ['openings', 'Opening trainer', 'i-lucide-book-marked', 'repertoire'],
  ['mistakes', 'Your mistakes', 'i-lucide-book-open', 'blunders review'],
  ['themes', 'Puzzle themes', 'i-lucide-shapes', 'tactics motifs'],
] as const
const PUZZLE_TABS: readonly [PuzzleTab, string, string, string][] = [
  ['train', 'Puzzle training', 'i-lucide-puzzle', 'tactics solve'],
  ['daily', 'Daily puzzle', 'i-lucide-calendar-days', 'today'],
  ['rush', 'Puzzle Storm, Streak and Rush', 'i-lucide-zap', 'timed race survival'],
  ['stats', 'Puzzle stats', 'i-lucide-chart-column', 'rating progress'],
  ['history', 'Puzzle history', 'i-lucide-history', 'solved attempts'],
]
const PAGE_KEYWORDS: Partial<Record<Page, string>> = {
  dashboard: 'start overview',
  computer: 'stockfish bot ai engine vs',
  local: 'otb friend two players pass and play clock',
  analysis: 'engine evaluate position fen pgn',
  history: 'past games archive',
  studies: 'pgn chapters notes',
  editor: 'setup position fen',
  online: 'lichess seek pairing',
  watch: 'tv broadcast',
  insights: 'stats openings performance',
  friends: 'following players',
  players: 'lookup profile user',
  settings: 'preferences options',
}
const ONLINE_CONTROLS = [
  [3, 2],
  [5, 0],
  [10, 0],
  [10, 5],
  [15, 10],
  [30, 0],
] as const
const COLORS = [
  ['white', 'White', 'i-lucide-circle'],
  ['black', 'Black', 'i-lucide-circle-dot'],
  ['random', 'Random side', 'i-lucide-dices'],
] as const

export const MODE_ROWS: { prefix: string; label: string; icon: string; kbds?: string[] }[] = [
  { prefix: '', label: 'Go to page', icon: 'i-lucide-arrow-right', kbds: ['meta', 'K'] },
  { prefix: '>', label: 'Run a command', icon: 'i-lucide-terminal', kbds: ['shift', 'meta', 'P'] },
  { prefix: '@', label: 'Find a player', icon: 'i-lucide-at-sign' },
  { prefix: '#', label: 'Change a setting', icon: 'i-lucide-sliders-horizontal' },
  { prefix: '?', label: 'Help', icon: 'i-lucide-circle-help' },
]

/** Builds the palette's groups for its current query, VS Code quick-open style. */
export function useQuickPick(query: Ref<string>) {
  const store = useKChessStore()
  // Page stores are created only when their command is used: their setup expects loaded data.
  const local = () => useLocalGameStore()
  const analysis = () => useAnalysisStore()
  // The same keys the Puzzles and Practice pages remember their tab in.
  const puzzleTab = useLocalStorage<PuzzleTab>('kchess:puzzle-tab', 'train')
  const practiceTab = useLocalStorage('kchess:practice-tab', 'coordinates')
  const recent = useLocalStorage<string[]>('kchess:palette-recent', [])

  const close = (): void => {
    store.searchOpen = false
  }
  const go = (page: Page): void => {
    close()
    store.choosePage(page)
  }
  const onOff = (on: boolean | undefined): string => (on ? 'On' : 'Off')

  const places = computed<QuickCommand[]>(() => [
    ...store.nav.map((item) => ({
      id: `page:${item.id}`,
      label: item.label,
      icon: item.icon,
      keywords: PAGE_KEYWORDS[item.id],
      hint: store.page === item.id ? 'Current' : undefined,
      run: () => go(item.id),
    })),
    {
      id: 'page:settings',
      label: 'Settings',
      icon: 'i-lucide-settings-2',
      keywords: PAGE_KEYWORDS.settings,
      kbds: ['meta', ','],
      hint: store.page === 'settings' ? 'Current' : undefined,
      run: () => go('settings'),
    },
    ...PUZZLE_TABS.map(([tab, label, icon, keywords]) => ({
      id: `puzzles:${tab}`,
      label,
      icon,
      detail: 'Puzzles',
      keywords,
      run: () => {
        puzzleTab.value = tab
        go('puzzles')
      },
    })),
    ...PRACTICE_TABS.map(([tab, label, icon, keywords]) => ({
      id: `practice:${tab}`,
      label,
      icon,
      detail: 'Practice',
      keywords,
      run: () => {
        practiceTab.value = tab
        go('practice')
      },
    })),
  ])

  const settingsCommands = computed<QuickCommand[]>(() => {
    const settings = store.ready ? store.settings : null
    const toggle = (
      key: 'soundEnabled' | 'showLegalMoves' | 'premove' | 'notificationsEnabled',
      label: string,
      icon: string,
      keywords: string,
    ): QuickCommand => ({
      id: `setting:${key}`,
      label,
      icon,
      keywords,
      hint: onOff(settings?.[key]),
      run: () => {
        if (settings) settings[key] = !settings[key]
      },
    })
    const quick = (
      key: 'zenMode' | 'blindfold' | 'cloudEval' | 'showOpeningName',
      label: string,
      icon: string,
      keywords: string,
      kbds?: string[],
    ): QuickCommand => ({
      id: `setting:${key}`,
      label,
      icon,
      keywords,
      kbds,
      hint: onOff(settings?.[key]),
      run: () => store.toggleSetting(key),
    })
    return [
      ...APPEARANCES.map((appearance) => ({
        id: `setting:appearance:${appearance}`,
        label: `Appearance: ${appearance === 'system' ? 'Match system' : appearance === 'light' ? 'Light' : 'Dark'}`,
        icon:
          appearance === 'dark'
            ? 'i-lucide-moon'
            : appearance === 'light'
              ? 'i-lucide-sun'
              : 'i-lucide-sun-moon',
        keywords: 'theme mode color scheme',
        hint: settings?.appearance === appearance ? 'Current' : undefined,
        run: () => {
          if (settings) settings.appearance = appearance
        },
      })),
      toggle('soundEnabled', 'Sound', 'i-lucide-volume-2', 'mute audio'),
      quick('zenMode', 'Zen mode', 'i-lucide-minimize', 'focus distraction', ['Z']),
      quick('blindfold', 'Blindfold', 'i-lucide-eye-off', 'hide pieces'),
      toggle('showLegalMoves', 'Show legal moves', 'i-lucide-locate', 'dots hints'),
      toggle('premove', 'Premoves', 'i-lucide-fast-forward', 'queue move'),
      quick('showOpeningName', 'Show opening name', 'i-lucide-book-marked', 'eco'),
      quick('cloudEval', 'Lichess cloud evaluation', 'i-lucide-cloud', 'analysis engine'),
      toggle('notificationsEnabled', 'Notifications', 'i-lucide-bell', 'alerts'),
      ...COORDINATE_MODES.map((mode) => ({
        id: `setting:coordinates:${mode}`,
        label: `Coordinates: ${mode === 'none' ? 'Hidden' : mode === 'inside' ? 'Inside the board' : 'Outside the board'}`,
        icon: 'i-lucide-grid-3x3',
        keywords: 'board labels files ranks',
        hint: settings?.coordinates === mode ? 'Current' : undefined,
        run: () => {
          if (settings) settings.coordinates = mode
        },
      })),
      ...SETTINGS_SECTIONS.map((section) => ({
        id: `settings:${section.id}`,
        label: section.label,
        icon: section.icon,
        detail: 'Settings',
        run: () => {
          go('settings')
          store.jumpToSection(section.id)
        },
      })),
    ]
  })

  /** Actions for the page in view; they come first in command mode. */
  const pageCommands = computed<QuickCommand[]>(() => {
    const zen = settingsCommands.value.find((command) => command.id === 'setting:zenMode')!
    const inProgress = store.localMoves.length > 0 && !store.localOver
    switch (store.page) {
      case 'computer':
        return [
          {
            id: 'computer:new',
            label: 'New game',
            icon: 'i-lucide-rotate-ccw',
            keywords: 'restart',
            run: () => store.startComputerGame(store.level, store.userColor),
          },
          ...(inProgress
            ? [
                {
                  id: 'computer:takeback',
                  label: 'Take back move',
                  icon: 'i-lucide-undo-2',
                  keywords: 'undo',
                  run: () => {
                    close()
                    store.takeback()
                  },
                },
                {
                  id: 'computer:resign',
                  label: 'Resign',
                  icon: 'i-lucide-flag',
                  keywords: 'give up',
                  run: () => {
                    close()
                    store.ask({
                      title: 'Resign this game?',
                      description: 'The game ends as a loss.',
                      label: 'Resign',
                      run: () => store.resign(),
                    })
                  },
                },
              ]
            : []),
          zen,
        ]
      case 'local':
        return [
          {
            id: 'local:flip',
            label: 'Flip board',
            icon: 'i-lucide-arrow-down-up',
            keywords: 'rotate orientation',
            run: () => {
              close()
              local().flip()
            },
          },
          ...(local().moves.length && !local().over
            ? [
                {
                  id: 'local:takeback',
                  label: 'Take back move',
                  icon: 'i-lucide-undo-2',
                  keywords: 'undo',
                  run: () => {
                    close()
                    local().takeback()
                  },
                },
              ]
            : []),
          zen,
        ]
      case 'analysis':
        return [
          {
            id: 'analysis:flip',
            label: 'Flip board',
            icon: 'i-lucide-arrow-down-up',
            keywords: 'rotate orientation',
            run: () => {
              close()
              const board = analysis()
              board.orientation = board.orientation === 'white' ? 'black' : 'white'
            },
          },
        ]
      case 'online':
        return [zen]
      default:
        return []
    }
  })

  const playCommands = computed<QuickCommand[]>(() => {
    const levels = store.ready ? store.settings.engineLevels : DEFAULT_ENGINE_LEVELS
    const stockfish = levels.flatMap((level) => {
      const info = engineLevelInfo(level)
      return COLORS.map(([color, colorLabel]) => ({
        id: `play:stockfish:${level}:${color}`,
        label: `Play Stockfish: ${info.label}`,
        detail: `${info.elo ? `~${info.elo}` : 'Full strength'} · ${colorLabel}`,
        icon: 'i-lucide-cpu',
        keywords: `computer bot engine ai ${colorLabel}`,
        color,
        run: () =>
          store.startComputerGame(
            level,
            color === 'random' ? (Math.random() < 0.5 ? 'white' : 'black') : color,
          ),
      }))
    })
    const connected = store.connectedAccounts.length > 0
    const online: QuickCommand[] = connected
      ? ONLINE_CONTROLS.filter(([minutes, increment]) =>
          canPlayOnline(minutes, increment, false),
        ).flatMap(([minutes, increment]) =>
          [false, true].map((rated) => ({
            id: `play:online:${minutes}+${increment}:${rated ? 'rated' : 'casual'}`,
            label: `Play online: ${minutes}+${increment}`,
            detail: `${perfFor(minutes, increment)} · ${rated ? 'Rated' : 'Casual'}`,
            icon: 'i-lucide-globe-2',
            keywords: 'lichess seek opponent',
            run: () => void store.startOnlineGame(minutes, increment, rated),
          })),
        )
      : []
    return [
      ...stockfish,
      ...online,
      {
        id: 'play:analysis',
        label: 'New analysis board',
        icon: 'i-lucide-microscope',
        keywords: 'empty start position',
        run: () => {
          analysis().load()
          go('analysis')
        },
      },
    ]
  })

  const appCommands = computed<QuickCommand[]>(() => [
    ...(store.connectedAccounts.length
      ? [
          {
            id: 'app:sync',
            label: 'Sync Lichess games',
            icon: 'i-lucide-refresh-cw',
            keywords: 'download update import',
            run: () => {
              close()
              void store.sync()
            },
          },
        ]
      : [
          {
            id: 'app:connect',
            label: 'Connect Lichess account',
            icon: 'i-lucide-log-in',
            keywords: 'sign in login oauth online',
            run: () => {
              close()
              void store.connect()
            },
          },
        ]),
    {
      id: 'app:sidebar',
      label: store.sidebarOpen ? 'Hide sidebar' : 'Show sidebar',
      icon: 'i-lucide-panel-left',
      keywords: 'toggle navigation',
      run: () => {
        close()
        store.sidebarOpen = !store.sidebarOpen
      },
    },
  ])

  const playerCommands = computed<QuickCommand[]>(() => {
    const connected = store.connectedAccounts.length > 0
    return store.trackedAccounts.flatMap((friend) => [
      {
        id: `player:${friend.username}`,
        label: `@${friend.username}`,
        detail: 'Profile',
        icon: 'i-lucide-user-round',
        keywords: 'following friend',
        run: () => {
          close()
          void navigateTo({ path: '/players', query: { name: friend.username } })
        },
      },
      ...(connected
        ? [false, true].map((rated) => ({
            id: `challenge:${friend.username}:${rated ? 'rated' : 'casual'}`,
            label: `Challenge @${friend.username}`,
            detail: rated ? 'Rated' : 'Casual',
            icon: 'i-lucide-swords',
            keywords: 'play game invite',
            run: () => store.challengeFriend(friend.username, rated),
          }))
        : []),
    ])
  })

  const all = computed(() => [
    ...places.value,
    ...pageCommands.value,
    ...playCommands.value,
    ...appCommands.value,
    ...settingsCommands.value,
    ...playerCommands.value,
  ])

  /** Something worth returning to, shown above recent entries. */
  const resume = computed<QuickCommand[]>(() => {
    const items: QuickCommand[] = []
    if (store.onlinePhase === 'playing' && store.page !== 'online')
      items.push({
        id: 'resume:online',
        label: 'Return to your online game',
        icon: 'i-lucide-globe-2',
        hint: 'In progress',
        run: () => go('online'),
      })
    if (store.localMoves.length && !store.localOver && store.page !== 'computer')
      items.push({
        id: 'resume:computer',
        label: 'Resume game vs Stockfish',
        detail: engineLevelInfo(store.level).label,
        icon: 'i-lucide-cpu',
        hint: 'In progress',
        run: () => go('computer'),
      })
    return items
  })

  const recentCommands = computed(() =>
    recent.value
      .map((id) => all.value.find((command) => command.id === id))
      .filter((command): command is QuickCommand => Boolean(command))
      .slice(0, 6),
  )

  function toItem(command: QuickCommand, ranges: [number, number][] = []): QuickPickItem {
    return {
      id: command.id,
      label: command.label,
      icon: command.icon,
      kbds: command.kbds,
      hint: command.hint,
      labelHtml: ranges.length ? highlightRanges(command.label, ranges) : escapeHtml(command.label),
      suffix: command.detail,
      suffixHtml: command.detail ? escapeHtml(command.detail) : undefined,
      onSelect: (event: Event) => {
        event.preventDefault()
        if (command.prefix !== undefined) {
          query.value = command.prefix
          return
        }
        recent.value = pushRecent(recent.value, command.id)
        command.run()
        // Every command leaves the palette unless it asked to stay (mode rows).
        close()
      },
    }
  }

  function group(id: string, label: string | undefined, commands: QuickCommand[]) {
    return commands.length
      ? { id, label, ignoreFilter: true, items: commands.map((c) => toItem(c)) }
      : null
  }

  function searched(id: string, label: string, text: string, commands: QuickCommand[], limit = 8) {
    const ranked = rankQuickPick(text, commands).slice(0, limit)
    return ranked.length
      ? {
          id,
          label,
          ignoreFilter: true,
          items: ranked.map(({ item, match }) => toItem(item, match.ranges)),
          score: ranked[0]!.match.score,
        }
      : null
  }

  const parsed = computed(() => parseQuickPick(query.value))
  const mode = computed<QuickPickMode>(() => parsed.value.mode)

  const groups = computed<CommandPaletteGroup<QuickPickItem>[]>(() => {
    const { mode, text } = parsed.value
    const compact = <T>(list: (T | null)[]): T[] =>
      list.filter((entry): entry is T => Boolean(entry))

    if (mode === 'help') {
      return compact([
        group(
          'help',
          undefined,
          MODE_ROWS.filter((row) => row.prefix !== '?').map((row) => ({
            id: `mode:${row.prefix || 'go'}`,
            label: row.label,
            detail: row.prefix || undefined,
            icon: row.icon,
            kbds: row.kbds,
            prefix: row.prefix,
            run: () => {},
          })),
        ),
      ])
    }

    if (mode === 'commands') {
      if (!text) {
        const userColor = store.userColor
        return compact([
          group('page', 'This page', pageCommands.value),
          group(
            'play',
            'Play',
            playCommands.value.filter(
              (command) =>
                !command.id.startsWith('play:stockfish') || command.id.endsWith(`:${userColor}`),
            ),
          ),
          group('app', 'App', appCommands.value),
        ])
      }
      return compact([
        searched('page', 'This page', text, pageCommands.value),
        searched('play', 'Play', text, playCommands.value, 12),
        searched('app', 'App', text, appCommands.value),
        searched('settings', 'Settings', text, settingsCommands.value),
      ])
    }

    if (mode === 'settings') {
      return text
        ? compact([searched('settings', 'Settings', text, settingsCommands.value, 20)])
        : compact([
            group(
              'toggles',
              'Quick settings',
              settingsCommands.value.filter((command) => !command.id.startsWith('settings:')),
            ),
            group(
              'sections',
              'Settings pages',
              settingsCommands.value.filter((command) => command.id.startsWith('settings:')),
            ),
          ])
    }

    if (mode === 'players') {
      const lookup: QuickCommand[] = text
        ? [
            {
              id: `lookup:${text}`,
              label: `Look up “${text}” on Lichess`,
              icon: 'i-lucide-user-search',
              run: () => {
                close()
                void navigateTo({ path: '/players', query: { name: text.replace(/^@/, '') } })
              },
            },
          ]
        : []
      return compact([
        text
          ? searched('players', 'Following', text, playerCommands.value, 12)
          : group('players', 'Following', playerCommands.value),
        group('lookup', undefined, lookup),
        !text && !playerCommands.value.length
          ? group('lookup', undefined, [
              {
                id: 'page:players',
                label: 'Type a Lichess username to look it up',
                icon: 'i-lucide-user-search',
                run: () => go('players'),
              },
            ])
          : null,
      ])
    }

    if (!text) {
      const modes = MODE_ROWS.slice(1).map((row) => ({
        id: `mode:${row.prefix}`,
        label: row.label,
        detail: row.prefix,
        icon: row.icon,
        kbds: row.kbds,
        prefix: row.prefix,
        run: () => {},
      }))
      const recentList = recentCommands.value.filter(
        (command) => !resume.value.some((item) => item.id === command.id),
      )
      const firstRecent = recentList[0]
      return compact([
        group('modes', undefined, modes),
        group('resume', undefined, resume.value),
        recentList.length
          ? group('recent', undefined, [
              { ...firstRecent!, hint: firstRecent!.hint ?? 'recently used' },
              ...recentList.slice(1),
            ])
          : null,
        group(
          'pages',
          recentList.length ? 'Pages' : undefined,
          places.value.filter((command) => command.id.startsWith('page:')),
        ),
      ])
    }

    // No prefix: search everything, best group first.
    const results = compact([
      searched('places', 'Go to', text, places.value),
      searched('page', 'This page', text, pageCommands.value),
      searched('play', 'Play', text, playCommands.value),
      searched('app', 'Commands', text, appCommands.value),
      searched('settings', 'Settings', text, settingsCommands.value),
      searched('players', 'Following', text, playerCommands.value),
    ])
    return results.sort((a, b) => b.score - a.score)
  })

  return { groups, mode }
}
