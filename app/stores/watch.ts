import { defineStore } from 'pinia'
import { computed, ref, shallowRef } from 'vue'
import type {
  BroadcastGame,
  BroadcastSummary,
  BroadcastTourDetail,
  TvChannel,
  WatchFrame,
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

  const broadcastList = ref<BroadcastSummary[] | null>(null)
  const tour = ref<BroadcastTourDetail | null>(null)
  const roundId = ref('')
  const games = shallowRef<Map<string, BroadcastGame>>(new Map())
  const selectedGame = ref('')
  const roundEnded = ref(false)
  let roundSession = -1

  let offs: (() => void)[] = []
  function listen(): void {
    if (offs.length) return
    offs = [
      window.kchess.onWatch((next) => {
        if (next.session !== session) return
        frame.value = next
        frameAt.value = performance.now()
      }),
      window.kchess.onBroadcast((update) => {
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
      }),
    ]
  }
  function unlisten(): void {
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
    listen()
    error.value = ''
    frame.value = null
    target.value = next
    roundId.value = ''
    try {
      session = await window.kchess.watch(next)
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
      target.value = null
    }
  }
  async function stop(): Promise<void> {
    session = -1
    roundSession = -1
    target.value = null
    frame.value = null
    roundId.value = ''
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
    error.value = ''
    try {
      tour.value = await window.kchess.broadcastTour(id)
      const round =
        tour.value.rounds.find((r) => r.id === tour.value?.defaultRoundId) ??
        tour.value.rounds.find((r) => r.ongoing) ??
        tour.value.rounds.at(-1)
      if (round) await openRound(round.id)
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    }
  }
  async function openRound(id: string): Promise<void> {
    listen()
    target.value = null
    frame.value = null
    roundId.value = id
    games.value = new Map()
    selectedGame.value = ''
    roundEnded.value = false
    try {
      roundSession = await window.kchess.watchBroadcast(id)
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    }
  }
  function closeTour(): void {
    tour.value = null
    void stop()
  }
  const roundGames = computed(() => [...games.value.values()])
  const currentGame = computed(() => games.value.get(selectedGame.value))

  return {
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
    listen,
    unlisten,
  }
})
