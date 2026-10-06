import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import type {
  NewArena,
  TournamentDetail,
  TournamentList,
  TournamentSummary,
  TournamentSystem,
} from '../../src/shared/types'
import { useKChessStore } from './kchess'

/** Tournaments you have joined from KChess, by `system:id`, kept on this device. */
interface Joined {
  system: TournamentSystem
  id: string
  account: string
  name: string
  /** Stop keeping the event stream open for it after this time. */
  until: number
}

export const useTournamentStore = defineStore('tournaments', () => {
  const app = useKChessStore()
  const list = ref<TournamentList | null>(null)
  const loading = ref(false)
  const error = ref('')
  const selected = ref<{ system: TournamentSystem; id: string } | null>(null)
  const detail = ref<TournamentDetail | null>(null)
  const detailError = ref('')
  const needsReconnect = ref(false)
  const stored = useLocalStorage<Joined[]>('kchess:tournaments-joined', [])
  const joined = computed(() => stored.value.filter((entry) => entry.until > Date.now()))
  /** A tournament is under way for an account: pairings must be able to arrive. */
  const active = computed(() => joined.value.length > 0)

  async function refresh(): Promise<void> {
    loading.value = true
    error.value = ''
    try {
      list.value = await window.kchess.tournaments(app.activeOnlineAccount)
    } catch (cause) {
      error.value = cause instanceof Error ? cause.message : String(cause)
    } finally {
      loading.value = false
    }
  }
  let detailRequest = 0
  /** Leaderboard page of the open tournament (10 players each). */
  const page = ref(1)
  async function open(system: TournamentSystem, id: string, standingPage?: number): Promise<void> {
    const other = selected.value?.system !== system || selected.value?.id !== id
    if (other) {
      // Never show one event's details under another's name while it loads.
      detail.value = null
      page.value = 1
    }
    if (standingPage) page.value = standingPage
    selected.value = { system, id }
    const request = ++detailRequest
    detailError.value = ''
    try {
      const next = await window.kchess.tournament(system, id, app.activeOnlineAccount, page.value)
      if (request !== detailRequest) return
      detail.value = next
      // An arena says whether you are in it; forget it when it is over or you left.
      if (system === 'arena' && (next.status === 'finished' || !next.me || next.me.withdraw))
        forget(system, id)
    } catch (cause) {
      if (request === detailRequest)
        detailError.value = cause instanceof Error ? cause.message : String(cause)
    }
  }
  function close(): void {
    selected.value = null
    detail.value = null
    detailRequest++
  }
  function remember(
    summary: Pick<TournamentSummary, 'system' | 'id' | 'name'>,
    ends: number,
  ): void {
    const key = `${summary.system}:${summary.id}`
    stored.value = [
      ...stored.value.filter((entry) => `${entry.system}:${entry.id}` !== key),
      {
        system: summary.system,
        id: summary.id,
        name: summary.name,
        account: app.activeOnlineAccount,
        until: ends,
      },
    ]
  }
  function forget(system: TournamentSystem, id: string): void {
    stored.value = stored.value.filter((entry) => !(entry.system === system && entry.id === id))
  }
  function isJoined(system: TournamentSystem, id: string): boolean {
    return joined.value.some((entry) => entry.system === system && entry.id === id)
  }
  async function join(summary: TournamentSummary, password?: string): Promise<boolean> {
    needsReconnect.value = false
    const result = await app.runAction(() =>
      window.kchess.joinTournament(summary.system, summary.id, app.activeOnlineAccount, password),
    )
    if (!result) return false
    if (result !== true) {
      needsReconnect.value = true
      return false
    }
    // Arenas end at `finishesAt`; Swiss events have no fixed end, so allow a day.
    remember(summary, summary.finishesAt ?? Date.now() + 86_400_000)
    app.notifyInfo(
      summary.system === 'arena'
        ? `Joined ${summary.name}. Games open here automatically when you are paired.`
        : `Joined ${summary.name}. Each round's game opens here automatically.`,
    )
    if (selected.value?.id === summary.id) void open(summary.system, summary.id)
    return true
  }
  async function leave(summary: Pick<TournamentSummary, 'system' | 'id'>): Promise<void> {
    const result = await app.runAction(() =>
      window.kchess.leaveTournament(summary.system, summary.id, app.activeOnlineAccount),
    )
    if (!result) return
    if (result !== true) {
      needsReconnect.value = true
      return
    }
    forget(summary.system, summary.id)
    if (selected.value?.id === summary.id) void open(summary.system, summary.id)
  }
  /** Creates an arena run by the active account, then shows it. */
  async function create(
    arena: NewArena,
  ): Promise<{ created?: TournamentSummary; error?: string; reconnect?: boolean }> {
    needsReconnect.value = false
    try {
      const result = await window.kchess.createTournament(app.activeOnlineAccount, arena)
      if ('needsReconnect' in result) {
        needsReconnect.value = true
        return { reconnect: true }
      }
      await refresh()
      return { created: result }
    } catch (cause) {
      return { error: cause instanceof Error ? cause.message : String(cause) }
    }
  }
  return {
    list,
    page,
    loading,
    error,
    selected,
    detail,
    detailError,
    needsReconnect,
    joined,
    active,
    refresh,
    open,
    close,
    join,
    leave,
    create,
    isJoined,
  }
})
