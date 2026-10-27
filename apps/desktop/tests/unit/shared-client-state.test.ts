import { afterEach, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { desktop, deferred, emptyProgress } from './fixtures'
import { useLocalGameStore } from '../../app/stores/local'
import { useKChessStore } from '../../app/stores/kchess'
import { usePuzzleStore } from '../../app/stores/puzzles'
import { useStudyStore } from '../../app/stores/studies'
import { useTournamentStore } from '../../app/stores/tournaments'
import { flushSavedDocuments, loadLibrary, setLibraryRefresher } from '../../app/utils/library'
import { STANDARD_SETUP } from '@kchess/rules/variant'
import { INITIAL_FEN } from '@kchess/rules/position'
import type { ProfileChange, PuzzleDbProgress, TournamentDetail } from '@kchess/contracts/types'
import type { RepertoireMisses } from '@kchess/rules/library'
import profileSyncPlugin from '../../app/plugins/profileSync.client'
import TournamentEvent from '../../app/components/TournamentEvent.vue'

vi.hoisted(() => {
  ;(globalThis as Record<string, unknown>).defineNuxtPlugin = (setup: unknown) => setup
})
afterEach(() => {
  setLibraryRefresher(undefined)
  vi.useRealTimers()
})

// Two windows share one profile's core (docs/shared-core.md); the 27 October f1e65a0 audit.

it('quitting a window that never changed its session keeps another window’s save', async () => {
  const { api } = desktop()
  await loadLibrary()
  const local = useLocalGameStore()
  expect(local.moves).toEqual([])
  await api.saveSession('local', {
    setup: STANDARD_SETUP,
    moves: ['e2e4'],
    clock: null,
    times: null,
    result: null,
  })
  await flushSavedDocuments()
  expect((await api.library()).sessions.local?.moves).toEqual(['e2e4'])
  // This window's own move is still saved at quit.
  local.start()
  expect(local.move('d2d4')).toBe(true)
  await flushSavedDocuments()
  expect((await api.library()).sessions.local?.moves).toEqual(['d2d4'])
})

it.each(['done', 'cancelled', 'failed'] as const)(
  'a window watching another’s puzzle download leaves busy when it ends (%s)',
  async (phase) => {
    let progress: (value: PuzzleDbProgress) => void = () => {}
    let busy = true
    desktop({
      puzzleDbStatus: async () => ({ installed: true, busy, count: 20, bytes: 100 }),
      onPuzzleDbProgress: (callback) => {
        progress = callback
        return () => {}
      },
    })
    await useKChessStore().init()
    const puzzles = usePuzzleStore()
    await puzzles.refreshDb()
    expect(puzzles.dbBusy).toBe(true)
    // The service reports the end before it stops counting the download as running.
    progress({ phase, received: 100, kept: 20 })
    await flushPromises()
    expect(puzzles.dbBusy, phase).toBe(false)
    busy = false
    await puzzles.refreshDb()
    expect(puzzles.dbBusy, phase).toBe(false)
  },
)

it('a late repertoire answer does not restore misses cleared after it', async () => {
  const { api } = desktop()
  const studies = useStudyStore()
  const real = api.recordRepertoireMiss
  const gate = deferred<RepertoireMisses>()
  let committed: RepertoireMisses | undefined
  api.recordRepertoireMiss = async (key, fen) => {
    committed = await real(key, fen)
    return gate.promise
  }
  studies.recordMiss('study:white', INITIAL_FEN)
  await flushPromises()
  studies.clearMisses('study:white')
  await flushPromises()
  expect(studies.misses).toEqual({})
  gate.resolve(committed!)
  await flushPromises()
  expect(studies.misses).toEqual({})
  expect((await api.library()).repertoireMisses).toEqual({})
})

it('puzzle history shows another window’s attempts and a reconnected core’s log', async () => {
  let changed: (value: ProfileChange) => void = () => {}
  let reconnected: () => void = () => {}
  let attempts = 0
  const read = vi.fn(async () => {
    const value = emptyProgress()
    value.all.attempts = attempts
    return value
  })
  desktop({
    puzzleProgress: read,
    onProfileChanged: (callback) => {
      changed = callback
      return () => {}
    },
    onCoreReconnected: (callback) => {
      reconnected = callback
      return () => {}
    },
  })
  await useKChessStore().init()
  ;(profileSyncPlugin as unknown as () => void)()
  const puzzles = usePuzzleStore()
  await puzzles.loadHistory()
  attempts = 1
  changed({ areas: ['puzzles'], origin: 99 })
  await flushPromises()
  await puzzles.loadHistory()
  expect(puzzles.history?.all.attempts).toBe(1)
  attempts = 2
  reconnected()
  await flushPromises()
  await puzzles.loadHistory()
  expect(puzzles.history?.all.attempts).toBe(2)
})

it('a history read started before an attempt was saved is read again', async () => {
  const gate = deferred<ReturnType<typeof emptyProgress>>()
  const saved = emptyProgress()
  saved.all.attempts = 1
  const read = vi.fn(() => gate.promise)
  desktop({ puzzleProgress: read, recordPuzzleAttempt: async () => saved })
  await useKChessStore().init()
  const puzzles = usePuzzleStore()
  const reading = puzzles.loadHistory()
  await puzzles.loadNext()
  await puzzles.report(true)
  puzzles.end()
  await flushPromises()
  expect(puzzles.progress?.all.attempts).toBe(1)
  gate.resolve(emptyProgress())
  await reading
  read.mockResolvedValue(saved)
  await puzzles.loadHistory()
  expect(read).toHaveBeenCalledTimes(2)
  expect(puzzles.history?.all.attempts).toBe(1)
})

it('puzzle history is read again on a new local day', async () => {
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(new Date(2026, 9, 27, 23, 59))
  const read = vi.fn(async () => emptyProgress())
  desktop({ puzzleProgress: read })
  await useKChessStore().init()
  const puzzles = usePuzzleStore()
  await puzzles.loadHistory()
  await puzzles.loadHistory()
  expect(read).toHaveBeenCalledTimes(1)
  vi.setSystemTime(new Date(2026, 9, 28, 0, 1))
  await puzzles.loadHistory()
  expect(read).toHaveBeenCalledTimes(2)
})

it('switching account hides the previous account’s tournament membership', async () => {
  const detail = (account: string): TournamentDetail => ({
    id: 'Arena001',
    system: 'arena',
    name: 'Blitz',
    status: 'started',
    variant: 'standard',
    variantName: 'Standard',
    rated: true,
    clock: { limit: 180, increment: 0 },
    nbPlayers: 2,
    startsAt: 0,
    playable: true,
    standing: [],
    ...(account === 'Alice' ? { me: { withdraw: false, gameId: 'Game0001' } } : {}),
  })
  const late = deferred<TournamentDetail>()
  const tournament = vi.fn(async (_system: string, _id: string, account: string) =>
    account === 'Alice' && tournament.mock.calls.length > 1 ? late.promise : detail(account),
  )
  desktop({ tournament })
  const app = useKChessStore()
  await app.init()
  app.setOnlineAccount('Alice')
  const wrapper = mount(TournamentEvent, {
    props: { system: 'arena', id: 'Arena001' },
    global: {
      stubs: {
        UButton: { template: '<button><slot /></button>' },
        UBadge: { template: '<span><slot /></span>' },
        UIcon: true,
        UInput: true,
        UAlert: true,
        PlayerLink: true,
        ChessBoard: true,
      },
    },
  })
  await flushPromises()
  expect(wrapper.text()).toContain('You are in')
  // A reply for Alice still on its way when Bob is selected is not shown as Bob's.
  void useTournamentStore().open('arena', 'Arena001')
  app.setOnlineAccount('Bob')
  await nextTick()
  expect(wrapper.text()).not.toContain('You are in')
  expect(wrapper.text()).not.toContain('Go to your game')
  await flushPromises()
  expect(tournament).toHaveBeenLastCalledWith('arena', 'Arena001', 'Bob', 1)
  late.resolve(detail('Alice'))
  await flushPromises()
  expect(wrapper.text()).not.toContain('You are in')
  wrapper.unmount()
})
