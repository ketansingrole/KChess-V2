import { effectScope, nextTick, ref } from 'vue'
import { expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { INITIAL_FEN } from '@kchess/rules/position'
import { analysisContext } from '@kchess/rules/analysisContext'
import type { AnalysisRequest, AnalysisUpdate, BroadcastGame } from '@kchess/contracts/types'
import { useBroadcastEvaluations } from '../../app/composables/useBroadcastEvaluations'
import EvalBar from '../../app/components/EvalBar.vue'
import { desktop } from './fixtures'

const game = (id: string): BroadcastGame => ({
  id,
  name: 'Round 1',
  white: { name: 'White' },
  black: { name: 'Black' },
  startFen: INITIAL_FEN,
  fen: INITIAL_FEN,
  moves: [],
  result: '*',
  ongoing: true,
  pgn: '*',
})

function setup() {
  vi.useFakeTimers()
  let receive!: (value: AnalysisUpdate) => void
  const startAnalysis = vi.fn<(request: AnalysisRequest) => Promise<number>>(async () => 1)
  const stopAnalysis = vi.fn(async () => {})
  const off = vi.fn()
  desktop({
    startAnalysis,
    stopAnalysis,
    onAnalysis: (fn) => {
      receive = fn
      return off
    },
  })
  const games = ref([game('one'), game('two')])
  const detail = ref<AnalysisRequest | null>(null)
  const identity = ref('bundled')
  const scope = effectScope()
  const engine = scope.run(() =>
    useBroadcastEvaluations(
      () => detail.value,
      () => games.value,
      () => identity.value,
    ),
  )!
  function emit(request: AnalysisRequest, cp: number, depth = 12) {
    receive({
      id: 1,
      clientId: request.clientId,
      fen: request.fen,
      context: analysisContext(request.rootFen ?? request.fen, request.moves),
      depth,
      lines: [{ rank: 1, depth, cp, pv: ['e2e4'] }],
      done: false,
    })
  }
  return { engine, scope, games, detail, identity, startAnalysis, stopAnalysis, off, emit }
}

it('samples boards sequentially, rejects late results and keeps each score with its position', async () => {
  const s = setup()
  await vi.advanceTimersByTimeAsync(100)
  expect(s.startAnalysis).toHaveBeenCalledTimes(1)
  const first = s.startAnalysis.mock.calls[0]![0] as unknown as AnalysisRequest
  expect(first).toMatchObject({ rootFen: INITIAL_FEN, moves: [], lines: 1 })
  s.emit(first, 150)
  await nextTick()
  expect(s.engine.evaluation(s.games.value[0]!)?.cp).toBe(150)
  await vi.advanceTimersByTimeAsync(100)
  const second = s.startAnalysis.mock.calls[1]![0] as unknown as AnalysisRequest
  s.emit(first, -800)
  expect(s.engine.evaluation(s.games.value[1]!)).toBeUndefined()
  s.emit(second, -80)
  await nextTick()
  expect(s.engine.evaluation(s.games.value[1]!)?.cp).toBe(-80)
  await vi.advanceTimersByTimeAsync(3000)
  expect(s.startAnalysis).toHaveBeenCalledTimes(2)
  s.games.value[0] = {
    ...s.games.value[0]!,
    moves: ['e2e4'],
    fen: 'rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1',
  }
  expect(s.engine.evaluation(s.games.value[0]!)).toBeUndefined()
  await vi.advanceTimersByTimeAsync(100)
  expect(s.startAnalysis).toHaveBeenCalledTimes(3)
  s.scope.stop()
  expect(s.off).toHaveBeenCalledOnce()
})

it('bounds search time, skips final games and distinguishes unavailable scores from equal positions', async () => {
  const s = setup()
  s.games.value = [game('one'), { ...game('two'), ongoing: false, result: '1-0' }]
  await vi.advanceTimersByTimeAsync(100)
  const first = s.startAnalysis.mock.calls[0]![0] as unknown as AnalysisRequest
  s.emit(first, 0, 4)
  const stopped = s.stopAnalysis.mock.calls.length
  await vi.advanceTimersByTimeAsync(1500)
  expect(s.engine.evaluation(s.games.value[0]!)?.cp).toBe(0)
  expect(s.startAnalysis).toHaveBeenCalledTimes(1)
  expect(s.stopAnalysis).toHaveBeenCalledTimes(stopped + 1)
  s.identity.value = 'native'
  await vi.advanceTimersByTimeAsync(1600)
  expect(s.engine.evaluation(s.games.value[0]!)).toBeUndefined()
  expect(s.engine.unavailable(s.games.value[0]!)).toBe(true)
  s.scope.stop()
})

it('gives the open board priority, cancels when disabled and does not start after disposal', async () => {
  const s = setup()
  await vi.advanceTimersByTimeAsync(100)
  const grid = s.startAnalysis.mock.calls[0]![0] as unknown as AnalysisRequest
  s.detail.value = { fen: INITIAL_FEN, rootFen: INITIAL_FEN, moves: [], lines: 3, infinite: true }
  await vi.advanceTimersByTimeAsync(100)
  expect(s.startAnalysis).toHaveBeenLastCalledWith(
    expect.objectContaining({ lines: 3, infinite: true }),
  )
  s.emit(grid, 1000)
  expect(s.engine.update.value).toBeNull()
  s.games.value = []
  s.detail.value = null
  await nextTick()
  expect(s.engine.busy.value).toBe(false)
  s.games.value = [game('three')]
  s.scope.stop()
  await vi.advanceTimersByTimeAsync(3000)
  expect(s.startAnalysis).toHaveBeenCalledTimes(2)
})

it('shows missing evaluations without suggesting equality, and renders scores and final results', async () => {
  const wrapper = mount(EvalBar, { props: { orientation: 'white' } })
  expect(wrapper.get('[role="meter"]').attributes('aria-valuenow')).toBeUndefined()
  expect(wrapper.find('.eval-white').exists()).toBe(false)
  await wrapper.setProps({ line: { rank: 1, depth: 12, cp: 150, pv: [] } })
  expect(Number(wrapper.attributes('aria-valuenow'))).toBeGreaterThan(50)
  expect(wrapper.attributes('aria-valuetext')).toBe('+1.5')
  await wrapper.setProps({ orientation: 'black', result: '0-1' })
  expect(wrapper.attributes('aria-valuenow')).toBe('0')
  expect(wrapper.classes()).toContain('flipped')
  await wrapper.setProps({ result: '½-½' })
  expect(wrapper.attributes('aria-valuenow')).toBe('50')
  wrapper.unmount()
})
