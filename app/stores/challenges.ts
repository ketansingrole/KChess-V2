import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import type { ChallengeInfo, DeclineReason, LobbyState, OngoingGame } from '../../src/shared/types'
import { useKChessStore } from './kchess'
import { useTournamentStore } from './tournaments'

/**
 * Challenges to and from the connected accounts, the idle event stream that delivers them, and
 * the games in progress (correspondence ones are checked for "your turn" on a timer).
 */
export const useChallengeStore = defineStore('challenges', () => {
  const app = useKChessStore()
  const list = ref<ChallengeInfo[]>([])
  const lobby = ref<LobbyState | null>(null)
  const ongoing = ref<OngoingGame[]>([])
  const ongoingLoaded = ref(false)
  const ongoingError = ref('')
  const busyId = ref('')
  /** Turn flags seen at the last check, to notify only when it becomes your turn. */
  const lastTurn = new Map<string, boolean>()
  const incoming = computed(() => list.value.filter((c) => c.direction === 'in'))
  const outgoing = computed(() => list.value.filter((c) => c.direction === 'out'))
  const correspondence = computed(() => ongoing.value.filter((g) => g.speed === 'correspondence'))
  const myTurnCount = computed(() => correspondence.value.filter((g) => g.isMyTurn).length)

  let offs: (() => void)[] = []
  let started = false

  async function refreshOngoing(): Promise<void> {
    if (!app.connectedAccounts.length) {
      ongoing.value = []
      return
    }
    try {
      const games = await window.kchess.ongoingGames()
      for (const game of games) {
        const before = lastTurn.get(game.gameId)
        if (
          game.speed === 'correspondence' &&
          game.isMyTurn &&
          before === false &&
          game.gameId !== app.onlineId
        )
          void window.kchess
            .notify({
              kind: 'opponentMove',
              title: `Your move against ${game.opponent.name}`,
              body: `Correspondence game · @${game.account}`,
            })
            .catch(() => undefined)
        lastTurn.set(game.gameId, game.isMyTurn)
      }
      ongoing.value = games
      ongoingError.value = ''
    } catch (cause) {
      ongoingError.value = cause instanceof Error ? cause.message : String(cause)
    } finally {
      ongoingLoaded.value = true
    }
  }

  const pollMs = ref(5 * 60_000)
  const poll = useIntervalFn(() => void refreshOngoing(), pollMs, { immediate: false })
  watch(
    () => (app.ready ? app.settings.correspondencePoll : 0),
    (minutes) => {
      poll.pause()
      if (!minutes || !started) return
      pollMs.value = Math.max(1, minutes) * 60_000
      poll.resume()
    },
  )

  /** Which account's event stream should stay open while idle. */
  const tournaments = useTournamentStore()
  // Tournament pairings arrive on the same stream, so it stays open while you are in one.
  const lobbyAccount = computed(() =>
    app.ready && (app.settings.receiveChallenges || tournaments.active)
      ? app.activeOnlineAccount
      : '',
  )
  watch(lobbyAccount, (account) => {
    if (!started) return
    void window.kchess.stayConnected(account).catch(() => undefined)
    if (!account) lobby.value = null
  })

  function start(): void {
    if (started) return
    started = true
    offs = [
      window.kchess.onChallenges((next) => (list.value = next)),
      window.kchess.onLobbyState((state) => (lobby.value = state)),
      window.kchess.onOngoingChanged(() => void refreshOngoing()),
    ]
    void window.kchess
      .challenges()
      .then((next) => (list.value = next))
      .catch(() => undefined)
    void window.kchess.stayConnected(lobbyAccount.value).catch(() => undefined)
    void refreshOngoing()
    if (app.settings.correspondencePoll) {
      pollMs.value = app.settings.correspondencePoll * 60_000
      poll.resume()
    }
  }
  function stop(): void {
    for (const off of offs) off()
    offs = []
    poll.pause()
    started = false
  }

  async function act(id: string, work: () => Promise<void>): Promise<boolean> {
    busyId.value = id
    try {
      return Boolean(
        await app.runAction(async () => {
          await work()
          return true
        }),
      )
    } finally {
      busyId.value = ''
    }
  }
  async function accept(challenge: ChallengeInfo): Promise<void> {
    const live = challenge.timeControl.type === 'clock'
    if (live) app.prepareForGame(challenge.account, challenge.id)
    const done = await act(challenge.id, () => window.kchess.acceptChallenge(challenge.id))
    if (done && live) app.selectPage('online')
    if (done && !live) {
      app.notifyInfo(
        `Accepted. Your game against ${challenge.opponent.name} is under Correspondence.`,
      )
      void refreshOngoing()
    }
  }
  function decline(challenge: ChallengeInfo, reason: DeclineReason = 'generic'): Promise<boolean> {
    return act(challenge.id, () => window.kchess.declineChallenge(challenge.id, reason))
  }
  function withdraw(challenge: ChallengeInfo): Promise<boolean> {
    return act(challenge.id, () => window.kchess.cancelChallenge(challenge.id))
  }
  /** The rematch offer for the game just finished, if the opponent sent one. */
  const rematchOffer = computed(() => {
    const last = app.lastGame
    return last ? incoming.value.find((c) => c.rematchOf === last.id) : undefined
  })

  return {
    list,
    incoming,
    outgoing,
    lobby,
    ongoing,
    ongoingLoaded,
    ongoingError,
    correspondence,
    myTurnCount,
    busyId,
    rematchOffer,
    refreshOngoing,
    start,
    stop,
    accept,
    decline,
    withdraw,
  }
})

/** "5+3", "3 days", "no clock". */
export function describeControl(challenge: Pick<ChallengeInfo, 'timeControl'>): string {
  const control = challenge.timeControl
  if (control.type === 'clock') {
    const minutes = control.limit / 60
    return `${Number.isInteger(minutes) ? minutes : minutes.toFixed(1)}+${control.increment}`
  }
  if (control.type === 'correspondence')
    return `${control.days} ${control.days === 1 ? 'day' : 'days'} per move`
  return 'No clock'
}
