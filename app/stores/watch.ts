import { defineStore } from 'pinia'
import { computed, ref, shallowRef } from 'vue'
import type {
  BroadcastGame,
  BroadcastUpdate,
  BroadcastSummary,
  BroadcastTourDetail,
  TvChannel,
  WatchFrame,
  WatchState,
} from '../../src/shared/types'

/** What is being watched: one TV channel or game, or one broadcast round. */
export const useWatchStore = defineStore('watch', () => {
  const channels = ref<TvChannel[]>([])
  const frame = ref<WatchFrame | null>(null)
  /** performance.now() when the frame arrived, to run the clock of the side to move. */
  const frameAt = ref(0)
  const target = ref<{ channel: string } | { gameId: string } | null>(null)
  const error = ref('')
  let session = -1
  let generation = 0
  const connection = ref<WatchState | null>(null)

  const broadcastList = ref<BroadcastSummary[] | null>(null)
  const tour = ref<BroadcastTourDetail | null>(null)
  const roundId = ref('')
  const games = shallowRef<Map<string, BroadcastGame>>(new Map())
  const selectedGame = ref('')
  const roundEnded = ref(false)
  let roundSession = -1

  let pendingKind: 'watch' | 'round' | undefined
  const early: (WatchState | WatchFrame | BroadcastUpdate)[] = []
  function state(next: WatchState): void {
    if (pendingKind === 'watch') {
      buffer(next)
      return
    }
    if (next.session !== session) return
    if (frame.value && connection.value?.phase === 'connected' && next.phase !== 'connected') {
      const current = frame.value
      const turn = current.fen.split(' ')[1] === 'b' ? 'blackClock' : 'whiteClock'
      if (!current.finished && current.lastMove && current[turn] !== undefined)
        current[turn] = Math.max(0, current[turn]! - (performance.now() - frameAt.value) / 1000)
      frameAt.value = performance.now()
    }
    connection.value = next
    if (next.message) error.value = next.message
  }
  function receiveFrame(next: WatchFrame): void {
    if (pendingKind === 'watch') {
      buffer(next)
      return
    }
    if (next.session !== session) return
    connection.value = { session, phase: 'connected' }
    frame.value = next
    frameAt.value = performance.now()
  }
  function receiveRound(update: BroadcastUpdate): void {
    if (pendingKind === 'round') {
      buffer(update)
      return
    }
    if (update.session !== roundSession) return
    if (update.games.length) {
      const next = new Map(games.value)
      for (const game of update.games) next.set(game.id, game)
      games.value = next
    }
    if (update.ended) {
      roundEnded.value = true
      if (update.error) error.value = update.error
    }
  }
  function buffer(update: WatchState | WatchFrame | BroadcastUpdate): void {
    early.push(update)
    if (early.length > 8) early.shift()
  }
  function clearPending(): void {
    pendingKind = undefined
    early.length = 0
  }
  function replayEarly(): void {
    const updates = early.splice(0)
    pendingKind = undefined
    for (const update of updates) {
      if ('phase' in update) state(update)
      else if ('games' in update) receiveRound(update)
      else receiveFrame(update)
    }
  }
  let offs: (() => void)[] = []
  function listen(): void {
    if (offs.length) return
    offs = [
      window.kchess.onWatchState(state),
      window.kchess.onWatch(receiveFrame),
      window.kchess.onBroadcast(receiveRound),
    ]
  }
  function unlisten(): void {
    generation++
    clearPending()
    for (const off of offs) off()
    offs = []
  }

  async function loadChannels(): Promise<void> {
    try {
      channels.value = await window.kchess.tvChannels()
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    }
  }
  async function watch(next: { channel: string } | { gameId: string }): Promise<void> {
    const request = ++generation
    clearPending()
    session = -1
    roundSession = -1
    connection.value = null
    listen()
    error.value = ''
    frame.value = null
    target.value = next
    pendingKind = 'watch'
    try {
      const id = await window.kchess.watch(next)
      if (request === generation) {
        session = id
        replayEarly()
      }
    } catch (cause) {
      if (request !== generation) return
      clearPending()
      error.value = cause instanceof Error ? cause.message : String(cause)
      target.value = null
    }
  }
  async function stop(): Promise<void> {
    generation++
    clearPending()
    connection.value = null
    session = -1
    roundSession = -1
    target.value = null
    frame.value = null
    await window.kchess.stopWatching().catch(() => undefined)
  }

  async function loadBroadcasts(): Promise<void> {
    try {
      broadcastList.value = await window.kchess.broadcasts()
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    }
  }
  async function openTour(id: string): Promise<void> {
    const stopping = stop()
    const request = generation
    await stopping
    if (request !== generation) return
    error.value = ''
    try {
      const found = await window.kchess.broadcastTour(id)
      if (request !== generation) return
      tour.value = found
      const round =
        tour.value.rounds.find((r) => r.id === tour.value?.defaultRoundId) ??
        tour.value.rounds.find((r) => r.ongoing) ??
        tour.value.rounds.at(-1)
      if (round) await openRound(round.id)
    } catch (cause) {
      if (request !== generation) return
      clearPending()
      error.value = cause instanceof Error ? cause.message : String(cause)
    }
  }
  /** Opens a round's live feed; `game` keeps that board selected once its feed arrives. */
  async function openRound(id: string, game = ''): Promise<void> {
    const request = ++generation
    clearPending()
    session = -1
    roundSession = -1
    connection.value = null
    error.value = ''
    listen()
    target.value = null
    frame.value = null
    roundId.value = id
    games.value = new Map()
    selectedGame.value = game
    roundEnded.value = false
    pendingKind = 'round'
    try {
      const next = await window.kchess.watchBroadcast(id)
      if (request === generation) {
        roundSession = next
        replayEarly()
      }
    } catch (cause) {
      if (request !== generation) return
      clearPending()
      error.value = cause instanceof Error ? cause.message : String(cause)
    }
  }
  /** Restarts the open tour's round feed after a tab switch, page return or reconnect. */
  async function resumeTour(): Promise<void> {
    if (!tour.value || !roundId.value || roundSession !== -1 || pendingKind === 'round') return
    await openRound(roundId.value, selectedGame.value)
  }
  function closeTour(): void {
    tour.value = null
    roundId.value = ''
    void stop()
  }
  const roundGames = computed(() => [...games.value.values()])
  const currentGame = computed(() => games.value.get(selectedGame.value))

  return {
    connection,
    channels,
    frame,
    frameAt,
    target,
    error,
    broadcastList,
    tour,
    roundId,
    roundGames,
    selectedGame,
    currentGame,
    roundEnded,
    loadChannels,
    watch,
    stop,
    loadBroadcasts,
    openTour,
    openRound,
    closeTour,
    resumeTour,
    listen,
    unlisten,
  }
})
