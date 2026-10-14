import { computed, ref, watch } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import type { Key } from '@lichess-org/chessground/types'
import type { Crosstable, TvPastGame, WatchPlayer } from '@kchess/contracts/types'
import { isVariant, type GameSetup } from '@kchess/rules/variant'
import { formatClock } from '@kchess/rules/clock'
import {
  lastMoveKeys,
  setupPgn,
  setupPositionAfter,
  setupSanHistory,
  fen as fenOf,
} from '@kchess/rules/chess'
import { useWatchStore } from '../stores/watch'
import { useAnalysisStore } from '../stores/analysis'
import { useKChessStore } from '../stores/kchess'

export const CHANNEL_ICONS: Record<string, string> = {
  best: 'i-lucide-crown',
  bullet: 'i-lucide-zap',
  blitz: 'i-lucide-flame',
  rapid: 'i-lucide-rabbit',
  classical: 'i-lucide-turtle',
  ultraBullet: 'i-lucide-rocket',
  chess960: 'i-lucide-dices',
  kingOfTheHill: 'i-lucide-mountain',
  threeCheck: 'i-lucide-layers',
  antichess: 'i-lucide-shield-off',
  atomic: 'i-lucide-atom',
  horde: 'i-lucide-grid-3x3',
  racingKings: 'i-lucide-flag',
  crazyhouse: 'i-lucide-house',
  bot: 'i-lucide-bot',
  computer: 'i-lucide-cpu',
}

export const SPEEDS: Record<string, { label: string; icon: string }> = {
  ultraBullet: { label: 'UltraBullet', icon: 'i-lucide-rocket' },
  bullet: { label: 'Bullet', icon: 'i-lucide-zap' },
  blitz: { label: 'Blitz', icon: 'i-lucide-flame' },
  rapid: { label: 'Rapid', icon: 'i-lucide-rabbit' },
  classical: { label: 'Classical', icon: 'i-lucide-turtle' },
  correspondence: { label: 'Correspondence', icon: 'i-lucide-mail' },
}

/** Channels that name a variant rather than a speed or a kind of player. */
export const NOT_VARIANTS = new Set(['best', 'bot', 'computer', ...Object.keys(SPEEDS)])

const ENDINGS: Record<string, string> = {
  mate: 'Checkmate',
  resign: 'Resignation',
  outoftime: 'Time out',
  timeout: 'Opponent left',
  draw: 'Draw',
  stalemate: 'Stalemate',
  aborted: 'Aborted',
  noStart: 'Did not start',
  cheat: 'Cheat detected',
  variantEnd: 'Variant win',
}

export interface TvBoardSide {
  name: string
  username: string | undefined
  icon: string
  clock: string | undefined
}

export interface TvBoardPlayers {
  top: TvBoardSide
  bottom: TvBoardSide
  topColor: 'white' | 'black'
  bottomColor: 'white' | 'black'
}

export interface TvFrameInfo {
  text: string
  icon: string
}

export interface TvScoreLine {
  label: string
  score: string
  games: number
}

/**
 * The shared TV/friends board: the live frame, its ticking clocks, the position
 * being looked at, orientation, move list, head-to-head and analysis handoff.
 */
export function useTvPosition() {
  const watcher = useWatchStore()
  const analysis = useAnalysisStore()
  const store = useKChessStore()
  const toast = useToast()

  const now = ref(performance.now())
  useIntervalFn(() => (now.value = performance.now()), 500)

  const frame = computed(() => watcher.frame)

  function clockOf(color: 'white' | 'black'): string | undefined {
    const current = frame.value
    if (!current) return undefined
    const base = color === 'white' ? current.whiteClock : current.blackClock
    if (base === undefined) return undefined
    const turn = current.fen.split(' ')[1] === 'b' ? 'black' : 'white'
    const running =
      watcher.connection?.phase === 'connected' &&
      !current.finished &&
      turn === color &&
      current.lastMove !== undefined
    const elapsed = running ? (now.value - watcher.frameAt) / 1000 : 0
    return formatClock(Math.max(0, base - elapsed) * 1000)
  }

  function label(player: WatchPlayer): string {
    return `${player.title ? `${player.title} ` : ''}${player.name}${player.rating ? ` (${player.rating})` : ''}`
  }

  const frameLast = computed<Key[] | undefined>(() => {
    const move = frame.value?.lastMove
    return move && !move.includes('@')
      ? [move.slice(0, 2) as Key, move.slice(2, 4) as Key]
      : undefined
  })

  /* Move list: known once Lichess's delayed export lines up with the feed. */
  const tvSetup = computed<GameSetup | null>(() => {
    const current = frame.value
    return current?.moves && current.startFen && isVariant(current.variant)
      ? { variant: current.variant, fen: current.startFen }
      : null
  })
  const tvMoves = computed(() => (tvSetup.value && frame.value?.moves) || [])
  const tvSan = computed(() => (tvSetup.value ? setupSanHistory(tvSetup.value, tvMoves.value) : []))
  /** The ply being looked at, or null to follow the live game. */
  const tvPly = ref<number | null>(null)
  const tvShownPly = computed(() => Math.min(tvPly.value ?? Infinity, tvMoves.value.length))
  const tvBrowsing = computed(
    () => tvSetup.value !== null && tvPly.value !== null && tvShownPly.value < tvMoves.value.length,
  )
  const tvFen = computed(() => {
    if (!frame.value) return ''
    const setup = tvSetup.value
    return tvBrowsing.value && setup
      ? fenOf(setupPositionAfter(setup, tvMoves.value.slice(0, tvShownPly.value)))
      : frame.value.fen
  })
  const tvLast = computed(() =>
    tvBrowsing.value ? lastMoveKeys(tvMoves.value.slice(0, tvShownPly.value)) : frameLast.value,
  )
  const flipped = ref(false)
  watch(
    () => frame.value?.gameId,
    () => {
      tvPly.value = null
      flipped.value = false
    },
  )
  const tvOrientation = computed(() => {
    const side = frame.value?.orientation ?? 'white'
    return flipped.value ? (side === 'white' ? 'black' : 'white') : side
  })
  function selectTvPly(ply: number): void {
    tvPly.value = ply >= tvMoves.value.length ? null : Math.max(0, ply)
  }
  const tvEmptyText = computed(() => {
    const current = frame.value
    if (current?.source !== 'tv') return 'Lichess does not send this game’s moves.'
    if (current.variant === 'crazyhouse') return 'No move list for Crazyhouse.'
    return 'Loading moves…'
  })

  const framePlayers = computed<TvBoardPlayers | null>(() => {
    const current = frame.value
    if (!current) return null
    const white: TvBoardSide = {
      name: label(current.white),
      username: realPlayer(current.white.name) ? current.white.name : undefined,
      icon: 'i-lucide-user',
      clock: clockOf('white'),
    }
    const black: TvBoardSide = {
      name: label(current.black),
      username: realPlayer(current.black.name) ? current.black.name : undefined,
      icon: 'i-lucide-user',
      clock: clockOf('black'),
    }
    return tvOrientation.value === 'white'
      ? { top: black, bottom: white, topColor: 'black' as const, bottomColor: 'white' as const }
      : { top: white, bottom: black, topColor: 'white' as const, bottomColor: 'black' as const }
  })

  /** "Black won · Checkmate", or "Live" while the game goes on. */
  const frameResult = computed(() => {
    const current = frame.value
    if (!current || !current.finished) return 'Live'
    const how = ENDINGS[current.status ?? ''] ?? current.status ?? ''
    if (!current.winner) return how || 'Finished'
    return `${current.winner === 'white' ? 'White' : 'Black'} won${how ? ` · ${how}` : ''}`
  })

  /** Lichess-style base time: 60 → "1", 30 → "½", 90 → "1.5". */
  function baseMinutes(seconds: number): string {
    const fractions: Record<number, string> = { 15: '¼', 30: '½', 45: '¾' }
    return fractions[seconds] ?? String(Math.round((seconds / 60) * 10) / 10)
  }
  /** "1+0 · Rated · Bullet", the line Lichess shows above a TV game. */
  const frameInfo = computed<TvFrameInfo | null>(() => {
    const current = frame.value
    if (!current) return null
    const speed = current.speed ? SPEEDS[current.speed] : undefined
    const variant =
      current.source === 'tv'
        ? current.channel && !NOT_VARIANTS.has(current.channel)
          ? watcher.channels.find((c) => c.key === current.channel)?.label
          : undefined
        : current.variant !== 'standard'
          ? current.variant
          : undefined
    const parts = [
      current.clock && `${baseMinutes(current.clock.initial)}+${current.clock.increment}`,
      current.rated === undefined ? undefined : current.rated ? 'Rated' : 'Casual',
      speed?.label ?? current.speed,
      variant,
    ].filter(Boolean)
    return {
      text: parts.join(' · '),
      icon: speed?.icon ?? CHANNEL_ICONS[current.channel ?? ''] ?? 'i-lucide-tv',
    }
  })

  /* ── Head to head ─────────────────────────────────────────────────────── */
  const headToHead = ref<{ key: string; table: Crosstable } | null>(null)
  const realPlayer = (name: string) =>
    name !== '?' && name !== 'Anonymous' && !name.startsWith('Stockfish level')
  const pairing = computed(() =>
    frame.value ? `${frame.value.white.name}|${frame.value.black.name}` : '',
  )
  watch(pairing, async (key) => {
    if (!key || headToHead.value?.key === key) return
    headToHead.value = null
    const [white = '', black = ''] = key.split('|')
    if (!realPlayer(white) || !realPlayer(black)) return
    try {
      const table = await window.kchess.crosstable(white, black)
      if (pairing.value === key && table.nbGames) headToHead.value = { key, table }
    } catch (cause) {
      console.warn('[watch] Head-to-head unavailable:', cause)
      // Optional context: the game is watchable without it.
    }
  })
  function points(value: number | undefined): string {
    const whole = Math.floor(value ?? 0)
    return (value ?? 0) % 1 ? `${whole || ''}½` : String(whole)
  }
  const scoreLines = computed<TvScoreLine[]>(() => {
    const found = headToHead.value
    const current = frame.value
    if (!found || !current) return []
    const score = (users: Record<string, number>) =>
      `${points(users[current.white.name.toLowerCase()])}–${points(users[current.black.name.toLowerCase()])}`
    return [
      { label: 'All time', score: score(found.table.users), games: found.table.nbGames },
      ...(found.table.matchup?.nbGames
        ? [
            {
              label: 'Current match',
              score: score(found.table.matchup.users),
              games: found.table.matchup.nbGames,
            },
          ]
        : []),
    ]
  })

  async function openPast(entry: TvPastGame): Promise<void> {
    try {
      const pgn = await window.kchess.exportGame(entry.gameId)
      if (!analysis.loadPgn(pgn)) throw new Error('That game could not be read.')
      analysis.orientation = entry.orientation
      analysis.origin = { white: entry.white.name, black: entry.black.name, gameId: entry.gameId }
      store.selectPage('analysis')
    } catch (cause) {
      console.warn('[watch] Could not open the game:', cause)
      toast.add({
        title: 'Could not open the game',
        description: cause instanceof Error ? cause.message : String(cause),
        color: 'error',
      })
    }
  }

  /** Opens the TV game so far in the analysis board, at the move being looked at. */
  async function analyseTv(): Promise<void> {
    const current = frame.value
    if (!current) return
    const origin = { white: current.white.name, black: current.black.name, gameId: current.gameId }
    if (tvSetup.value) {
      const pgn = setupPgn(tvSetup.value, tvMoves.value, {
        White: current.white.name,
        Black: current.black.name,
      })
      if (!analysis.loadPgn(pgn, tvShownPly.value)) return
      analysis.orientation = tvOrientation.value
      analysis.origin = origin
      store.selectPage('analysis')
    } else await openPast({ ...current, orientation: tvOrientation.value })
  }

  return {
    frame,
    label,
    tvSetup,
    tvMoves,
    tvSan,
    tvPly,
    tvShownPly,
    tvBrowsing,
    tvFen,
    tvLast,
    flipped,
    tvOrientation,
    selectTvPly,
    tvEmptyText,
    framePlayers,
    frameResult,
    frameInfo,
    scoreLines,
    openPast,
    analyseTv,
  }
}

export type TvPosition = ReturnType<typeof useTvPosition>
