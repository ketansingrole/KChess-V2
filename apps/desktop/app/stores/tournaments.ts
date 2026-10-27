import { defineStore } from 'pinia'
import { computed, ref, watch } from 'vue'
import type {
  NewArena,
  TournamentDetail,
  TournamentList,
  TournamentSummary,
  TournamentSystem,
} from '@kchess/contracts/types'
import { useKChessStore } from './kchess'
import { initialLibrary, libraryWrites } from '../utils/library'
import { errorText } from '../utils/format'
import type { JoinedTournament, LibrarySnapshot } from '@kchess/rules/library'

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
  const writes = libraryWrites()
  /**
   * The time `joined` is judged at, moved on when the next tournament ends: `Date.now()` alone is
   * not reactive, so an ended tournament stayed joined (and kept its stream wanted).
   */
  const clock = ref(Date.now())
  let expiry: ReturnType<typeof setTimeout> | undefined
  function scheduleExpiry(): void {
    clearTimeout(expiry)
    const now = Date.now()
    clock.value = now
    const next = Math.min(
      ...stored.value.map((entry) => entry.until).filter((until) => until > now),
    )
    // setTimeout holds at most 2^31 - 1 ms; a later end is checked again then.
    if (Number.isFinite(next))
      expiry = setTimeout(scheduleExpiry, Math.min(next - now + 1, 2 ** 31 - 1))
  }
  watch(stored, scheduleExpiry, { immediate: true })
  const joined = computed(() => stored.value.filter((entry) => entry.until > clock.value))
  /** A tournament is under way for an account: pairings must be able to arrive. */
  const active = computed(() => joined.value.length > 0)
  const sameAccount = (a: string, b: string) => a.toLowerCase() === b.toLowerCase()
  /**
   * The account taking part in a tournament, whose event stream must stay open until it ends:
   * the selected account if it is in one, otherwise the one that joined last. Selecting another
   * account must not drop the pairings of the one still playing.
   */
  const participant = computed(() => {
    const account = app.activeOnlineAccount
    if (account && joined.value.some((entry) => sameAccount(entry.account, account))) return account
    return joined.value.at(-1)?.account ?? ''
  })

  async function refresh(): Promise<void> {
    loading.value = true
    error.value = ''
    try {
      list.value = await window.kchess.tournaments(app.activeOnlineAccount)
    } catch (cause) {
      console.warn('[tournaments] loading tournaments failed:', cause)
      error.value = errorText(cause)
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
      if (joined.value.some((entry) => entry.system === system && entry.id === id))
        await reloadJoined()
    } catch (cause) {
      if (request === detailRequest) {
        console.warn('[tournaments] loading tournament failed:', cause)
        detailError.value = errorText(cause)
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
      stored.value = await writes.track(window.kchess.joinedTournaments())
    } catch (cause) {
      console.warn('[tournaments] loading joined tournaments failed:', cause)
    }
  }
  /**
   * Whether a game's tournament id is an arena or a Swiss event. Lichess game streams give only
   * the id, so look it up among the events KChess knows; arenas are the common case.
   */
  function systemOf(id: string): TournamentSystem {
    const known =
      stored.value.find((entry) => entry.id === id)?.system ??
      (selected.value?.id === id ? selected.value.system : undefined) ??
      list.value?.swiss.find((entry) => entry.id === id)?.system
    return known ?? 'arena'
  }
  /** Whether `account` (the selected account by default) is in this tournament. */
  function isJoined(
    system: TournamentSystem,
    id: string,
    account = app.activeOnlineAccount,
  ): boolean {
    return joined.value.some(
      (entry) => entry.system === system && entry.id === id && sameAccount(entry.account, account),
    )
  }
  async function join(summary: TournamentSummary, password?: string): Promise<boolean> {
    needsReconnect.value = false
    const result = await writes.track(
      app.runAction(() =>
        window.kchess.joinTournament(summary.system, summary.id, app.activeOnlineAccount, password),
      ),
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
    const result = await writes.track(
      app.runAction(() =>
        window.kchess.leaveTournament(summary.system, summary.id, app.activeOnlineAccount),
      ),
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
      return { error: errorText(cause) }
    }
  }
  /** Another KChess window joined or left a tournament; `since` is `revision()` when it was read. */
  function replaceFromLibrary(snapshot: LibrarySnapshot, since = writes.revision()): void {
    if (writes.fresh(since)) stored.value = snapshot.joinedTournaments
  }
  return {
    replaceFromLibrary,
    revision: writes.revision,
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
    participant,
    refresh,
    open,
    close,
    join,
    leave,
    create,
    isJoined,
    systemOf,
  }
})
