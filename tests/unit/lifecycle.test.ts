import { describe, it, expect, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { useKChessStore } from '../../app/stores/kchess'
import { usePuzzleStore } from '../../app/stores/puzzles'
import PuzzleBoard from '../../app/components/PuzzleBoard.vue'
import PuzzleRush from '../../app/components/PuzzleRush.vue'
import EndgameDrills from '../../app/components/EndgameDrills.vue'
import { desktop, deferred, puzzle, componentStubs, ChessBoardStub } from './fixtures'
import type {
  PuzzleSolveResult,
  NeedsReconnect,
  PuzzleDashboard,
  RunSaved,
  PuzzleSolveRequest,
  LichessGameStateEvent,
} from '../../src/shared/types'

async function puzzleStore(overrides: Parameters<typeof desktop>[0] = {}) {
  const bridge = desktop(overrides)
  await useKChessStore().init()
  return { store: usePuzzleStore(), ...bridge }
}

describe('puzzle lifecycle', () => {
  it('retains a failed verdict across a board remount and submits it once', async () => {
    const solve = vi.fn(async (_request: PuzzleSolveRequest) => ({}))
    const { store } = await puzzleStore({ puzzleSolve: solve })
    await store.loadNext()
    const board = mount(PuzzleBoard, {
      props: {
        puzzle,
        initialOutcome: store.attemptOutcome,
        onOutcome: (win: boolean) => void store.report(win),
      },
      global: { stubs: componentStubs },
    })
    board.findComponent(ChessBoardStub).vm.$emit('move', 'f2f3')
    await flushPromises()
    board.unmount()
    const remounted = mount(PuzzleBoard, {
      props: {
        puzzle,
        initialOutcome: store.attemptOutcome,
        onOutcome: (win: boolean) => void store.report(win),
      },
      global: { stubs: componentStubs },
    })
    expect(remounted.vm.outcome).toBe(false)
    remounted.findComponent(ChessBoardStub).vm.$emit('move', 'a1a8')
    await store.report(true)
    expect(solve).toHaveBeenCalledTimes(1)
    expect(solve.mock.calls[0]?.[0]).toMatchObject({ win: false })
    expect(store.session).toMatchObject({ solved: 0, failed: 1 })
    remounted.unmount()
  })

  it.each([{ needsReconnect: true }, { ratingDiff: 12 }, new Error('offline')])(
    'ignores a late report after starting a new practice puzzle (%j)',
    async (response) => {
      const pending = deferred<PuzzleSolveResult | NeedsReconnect>()
      const { store } = await puzzleStore({ puzzleSolve: () => pending.promise })
      await store.loadNext()
      const reporting = store.report(true)
      store.mode = 'practice'
      await store.loadNext()
      if (response instanceof Error) pending.reject(response)
      else pending.resolve(response)
      await reporting
      expect(store.phase).toBe('ready')
      expect(store.lastReport).toBeNull()
      expect(store.rating).toBe(1500)
      expect(store.session.ratingChange).toBe(0)
    },
  )

  it('ignores an old account result after switching accounts', async () => {
    const pending = deferred<PuzzleSolveResult>()
    const { store } = await puzzleStore({ puzzleSolve: () => pending.promise })
    await store.loadNext()
    const reporting = store.report(true)
    store.setAccount('Bob')
    await store.loadNext()
    pending.resolve({ ratingDiff: 20 })
    await reporting
    expect(store.rating).toBe(1500)
    expect(store.session).toMatchObject({ ratingChange: 0, solved: 0 })
  })

  it('clears old account statistics when reconnection is needed', async () => {
    const { store } = await puzzleStore({
      puzzleDashboard: async (account) =>
        account === 'Bob'
          ? { needsReconnect: true }
          : ({ days: 30, global: { nb: 42 }, themes: {} } as PuzzleDashboard),
    })
    await store.loadStats()
    expect(store.dashboard?.global.nb).toBe(42)
    store.setAccount('Bob')
    expect(store.dashboard).toBeNull()
    expect(store.profile).toBeNull()
    await store.loadStats()
    expect(store.stats.needsReconnect).toBe(true)
    expect(store.dashboard).toBeNull()
  })

  it('keeps a newer statistics period when an old request finishes', async () => {
    const pending = deferred<PuzzleDashboard>()
    const dashboard = vi
      .fn()
      .mockReturnValueOnce(pending.promise)
      .mockResolvedValue({ global: { nb: 7 }, themes: {} })
    const { store } = await puzzleStore({ puzzleDashboard: dashboard })
    const old = store.loadStats()
    store.dashboardDays = 7
    await store.loadStats()
    pending.resolve({ global: { nb: 30 }, themes: {} } as PuzzleDashboard)
    await old
    expect(store.dashboard?.global.nb).toBe(7)
  })

  it('invalidates a pending rating change when resetting the session', async () => {
    const pending = deferred<PuzzleSolveResult>()
    const { store } = await puzzleStore({ puzzleSolve: () => pending.promise })
    await store.loadNext()
    const reporting = store.report(true)
    store.resetSession()
    pending.resolve({ ratingDiff: 10 })
    await reporting
    expect(store.session.ratingChange).toBe(0)
  })
})

describe('mounted practice components', () => {
  it('invalidates Rush timers and saved results on restart and unmount', async () => {
    vi.useFakeTimers()
    const timer = vi.spyOn(globalThis, 'setTimeout')
    const saved = deferred<RunSaved>()
    await puzzleStore({ saveRun: () => saved.promise })
    usePuzzleStore().db = { installed: true, busy: false, count: 20, bytes: 0 }
    const RushBoard = defineComponent({
      name: 'PuzzleBoard',
      emits: ['done'],
      setup(_props, { expose }) {
        expose({ status: 'failed' })
        return {}
      },
      template: '<div />',
    })
    const wrapper = mount(PuzzleRush, {
      global: { stubs: { ...componentStubs, PuzzleBoard: RushBoard } },
    })
    const click = async (label: string) => {
      const button = wrapper.findAll('button').find((item) => item.text().includes(label))
      expect(button).toBeDefined()
      await button!.trigger('click')
      await flushPromises()
    }
    await click('Start')
    wrapper.findComponent(RushBoard).vm.$emit('done')
    const advance = timer.mock.calls.findLast((call) => call[1] === 750)?.[0] as () => void
    await click('End run')
    await click('Play again')
    advance()
    await flushPromises()
    expect(wrapper.text()).toContain('Puzzle 1')
    expect(wrapper.findComponent(RushBoard).exists()).toBe(true)
    saved.resolve({ summary: { kind: 'storm', best: {}, recent: [], total: 1 }, isBest: true })
    await flushPromises()
    expect(wrapper.text()).not.toContain('A new personal best!')
    wrapper.findComponent(RushBoard).vm.$emit('done')
    const late = timer.mock.calls.findLast((call) => call[1] === 750)?.[0] as () => void
    wrapper.unmount()
    expect(() => late()).not.toThrow()
  })

  it('sends cloneable move arrays for endgame hints and engine replies', async () => {
    const calls: string[][] = []
    await puzzleStore({
      bestMove: async (moves) => {
        calls.push(structuredClone(moves))
        return 'e8e7'
      },
    })
    const wrapper = mount(EndgameDrills, { global: { stubs: componentStubs } })
    await wrapper
      .findAll('button')
      .find((button) => button.text() === 'Hint')!
      .trigger('click')
    await flushPromises()
    wrapper.findComponent(ChessBoardStub).vm.$emit('move', 'd1d2')
    await flushPromises()
    expect(calls).toHaveLength(2)
    expect(calls[1]?.[0]).toBe('d1d2')
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
    wrapper.unmount()
  })
})

describe('live online game', () => {
  it('survives late seek results and ignores unrelated game events', async () => {
    const pending = deferred<{ url: string }>()
    const bridge = desktop({ startOnline: () => pending.promise })
    const store = useKChessStore()
    await store.init()
    const start = store.startOnline()
    bridge.emit({
      type: 'gameStart',
      game: {
        fullId: 'AbCd1234White',
        gameId: 'AbCd1234',
        color: 'white',
        opponent: { id: 'bob', username: 'Bob', rating: 1500 },
      },
    })
    pending.resolve({ url: 'https://lichess.org/AbCd1234' })
    await start
    expect(store.onlinePhase).toBe('playing')
    bridge.emit({
      type: 'gameFinish',
      game: { fullId: 'OTHER123White', gameId: 'OTHER123', status: { id: 30, name: 'mate' } },
    })
    const foreignState: LichessGameStateEvent & { id: string } = {
      type: 'gameState',
      id: 'OTHER123',
      moves: 'e2e4',
      status: 'mate',
      wtime: 0,
      btime: 0,
      winc: 0,
      binc: 0,
    }
    bridge.emit(foreignState)
    expect(store.onlinePhase).toBe('playing')
    expect(store.onlineHistory).toHaveLength(0)
    bridge.emit({
      type: 'gameFinish',
      game: { fullId: 'AbCd1234White', gameId: 'AbCd1234', status: { id: 30, name: 'mate' } },
    })
    expect(store.onlinePhase).toBe('finished')
  })

  it('clears seeking after cancellation or a failed request', async () => {
    let pending = deferred<{ seeking: boolean }>()
    desktop({ startOnline: () => pending.promise })
    const store = useKChessStore()
    await store.init()
    const start = store.startOnline()
    await store.stopOnline()
    pending.resolve({ seeking: true })
    await start
    expect(store.onlinePhase).toBe('idle')
    pending = deferred()
    const failed = store.startOnline()
    pending.reject(new Error('unavailable'))
    await failed
    expect(store.onlinePhase).toBe('idle')
  })
})
