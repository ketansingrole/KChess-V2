import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import type {
  NewArena,
  TournamentDetail,
  TournamentList,
  TournamentSummary,
  TournamentSystem,
} from '@kchess/contracts/types'
import { useKChessStore } from './kchess'
import { initialLibrary } from '../utils/library'
import type { JoinedTournament } from '@kchess/rules/library'

export const useTournamentStore = defineStore('tournaments', () => {
  const app = useKChessStore()
  const list = ref<TournamentList | null>(null)
  const loading = ref(false)
  const error = ref('')
  const selected = ref<{ system: TournamentSystem; id: string } | null>(null)
  const detail = ref<TournamentDetail | null>(null)
  const detailError = ref('')
  const needsReconnect = ref(false)
  /** Tournaments joined from KChess, as the core keeps them. */
  const stored = ref<JoinedTournament[]>(initialLibrary().joinedTournaments)
  const joined = computed(() => stored.value.filter((entry) => entry.until > Date.now()))
  /** A tournament is under way for an account: pairings must be able to arrive. */
  const active = computed(() => joined.value.length > 0)

  async function refresh(): Promise<void> {
    loading.value = true
    error.value = ''
    try {
      list.value = await window.kchess.tournaments(app.activeOnlineAccount)
    } catch (cause) {
      console.warn('[tournaments] loading tournaments failed:', cause)
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
      // The core forgets an arena that is over or that you left.
      if (isJoined(system, id)) await reloadJoined()
    } catch (cause) {
      if (request === detailRequest) {
        console.warn('[tournaments] loading tournament failed:', cause)
        detailError.value = cause instanceof Error ? cause.message : String(cause)
      }
    }
  }
  function close(): void {
    selected.value = null
    detail.value = null
    detailRequest++
  }
  async function reloadJoined(): Promise<void> {
    try {
      stored.value = await window.kchess.joinedTournaments()
    } catch (cause) {
      console.warn('[tournaments] loading joined tournaments failed:', cause)
    }
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
    await reloadJoined()
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
    await reloadJoined()
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
      console.warn('[tournaments] creating tournament failed:', cause)
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
