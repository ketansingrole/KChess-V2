import { expect, it, vi } from 'vitest'
import { defineComponent, nextTick } from 'vue'
import { flushPromises, mount, shallowMount } from '@vue/test-utils'
import type { Puzzle, ReviewRequest, StoredReview } from '@kchess/contracts/types'
import { INITIAL_FEN } from '@kchess/rules/position'
import { moveGlyph, nodeAt, pathOf } from '@kchess/rules/analysisTree'
import { replay, reviewKey } from '@kchess/rules/review'
import AnalysisPuzzle from '../../app/components/AnalysisPuzzle.vue'
import AnalysisReview from '../../app/components/AnalysisReview.vue'
import { useReviewStore } from '../../app/stores/review'
import AnalysisPage from '../../app/pages/analysis.vue'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useKChessStore } from '../../app/stores/kchess'
import { usePuzzleStore } from '../../app/stores/puzzles'
import { usePuzzleAnalysis, YOUR_MOVE } from '../../app/stores/puzzleAnalysis'
import { desktop } from './fixtures'

/** 1. e4 e5 2. Nf3 f6 3. Nxe5 fxe5: White finds 4. Qh5+ Ke7 5. Qxe5+ (and tried 4. d4). */
const game = ['e2e4', 'e7e5', 'g1f3', 'f7f6', 'f3e5', 'f6e5']
const fen = replay(INITIAL_FEN, game)[6]!.fen
const puzzle: Puzzle = {
  id: 'Ab12c',
  fen,
  lastMove: 'f6e5',
  solution: ['d1h5', 'e8e7', 'h5e5'],
  rating: 1650,
  themes: ['fork', 'short'],
  gameId: 'abcd1234',
}
const SOURCE =
  '[White "Ann"]\n[Black "Bob"]\n\n1. e4 e5 2. Nf3 f6 3. Nxe5 fxe5 4. Qh5+ Ke7 5. Qxe5+ 1-0\n'

async function board(api: Record<string, unknown> = {}) {
  const reviewRequest = vi.fn(async (request: ReviewRequest): Promise<StoredReview | null> => {
    if (request.moves.length !== 1) return null
    // The engine's view of 4. d4: from +3 to equal, a blunder.
    return {
      key: reviewKey(request.fen, request.moves),
      fen: request.fen,
      moves: request.moves,
      source: 'local',
      evals: [
        { cp: 300, depth: 18 },
        { cp: 0, depth: 18 },
      ],
      depth: 18,
      complete: true,
      updatedAt: 0,
    }
  })
  desktop({
    reviewGet: async () => null,
    reviewRequest,
    onReviewUpdate: () => () => {},
    onReviewStatus: () => () => {},
    reviewStatus: async () => ({ waiting: 0 }),
    onAnalysis: () => () => {},
    startAnalysis: async () => 0,
    stopAnalysis: async () => {},
    ...api,
  })
  const app = useKChessStore()
  await app.init()
  const analysis = useAnalysisStore()
  analysis.engineOn = false
  return { app, analysis, reviewRequest }
}
const card = () =>
  mount(AnalysisPuzzle, {
    global: {
      stubs: {
        UButton: {
          emits: ['click'],
          template: '<button @click="$emit(\'click\')"><slot /></button>',
        },
        UIcon: true,
      },
    },
  })
const button = (view: ReturnType<typeof card>, text: string) =>
  view.findAll('button').find((found) => found.text() === text)

// Analysing a puzzle put only its position on the board: no solution, nothing tried, no review.
it('studies a puzzle as its solution, with the move tried beside it, labelled', async () => {
  const { analysis, reviewRequest } = await board()
  expect(usePuzzleAnalysis().open(puzzle, [{ index: 0, uci: 'd2d4' }])).toBe(true)
  await flushPromises()
  expect(analysis.root.fen).toBe(fen)
  expect(analysis.mainline).toEqual(puzzle.solution)
  expect(analysis.path).toBe('')
  expect(analysis.orientation).toBe('white')
  expect(analysis.origin?.puzzle?.id).toBe('Ab12c')
  // The solution is reviewed like a game, and the tried move on its own.
  expect(reviewRequest).toHaveBeenCalledWith(expect.objectContaining({ moves: puzzle.solution }))
  expect(reviewRequest).toHaveBeenCalledWith({ fen, moves: ['d2d4'] })
  const tried = nodeAt(analysis.root, pathOf(['d2d4']))
  expect(tried.comments).toEqual([YOUR_MOVE])
  expect(moveGlyph(tried)?.glyph).toBe('??')
  // The solution's first move stays the main line.
  expect(analysis.root.children[0]!.uci).toBe('d1h5')
})

it('names the puzzle, jumps to the move tried, and tries it again', async () => {
  const { app, analysis } = await board()
  const selectPage = vi.spyOn(app, 'selectPage').mockImplementation(() => {})
  const retry = vi.spyOn(usePuzzleStore(), 'retry').mockImplementation(() => {})
  usePuzzleAnalysis().open(puzzle, [{ index: 0, uci: 'd2d4' }])
  const view = card()
  await nextTick()
  expect(view.text()).toContain('Puzzle')
  expect(view.text()).toContain('1650')
  expect(view.get('.puzzle-themes').text()).toBe('Fork')
  await view.get('.tried-move').trigger('click')
  expect(analysis.path).toBe(pathOf(['d2d4']))
  expect(view.get('.tried-move').text()).toBe('4. d4')
  await button(view, 'Try again')!.trigger('click')
  expect(retry).toHaveBeenCalledWith(expect.objectContaining({ id: 'Ab12c' }))
  expect(selectPage).toHaveBeenCalledWith('puzzles')
  view.unmount()
})

it('opens the game the puzzle comes from at its position, and goes back', async () => {
  const exportGame = vi.fn(async () => SOURCE)
  const { analysis } = await board({ exportGame })
  usePuzzleAnalysis().open(puzzle, [{ index: 0, uci: 'd2d4' }])
  const view = card()
  await nextTick()
  await button(view, 'Source game')!.trigger('click')
  await flushPromises()
  expect(exportGame).toHaveBeenCalledWith('abcd1234')
  expect(analysis.root.fen).toBe(INITIAL_FEN)
  expect(analysis.path).toBe(pathOf(game))
  expect(analysis.origin).toMatchObject({ gameId: 'abcd1234', puzzle: { id: 'Ab12c' } })
  expect(analysis.playerNames).toEqual({ white: 'Ann', black: 'Bob' })
  expect(button(view, 'Source game')).toBeUndefined()
  // Back from the game: the puzzle again, with the move tried.
  await button(view, 'Back to the puzzle')!.trigger('click')
  await flushPromises()
  expect(analysis.root.fen).toBe(fen)
  expect(view.get('.tried-move').text()).toBe('4. d4')
  // Analysis of your own on the board: switching asks first.
  analysis.play('d1h5')
  analysis.play('g7g6')
  const ask = vi.spyOn(useKChessStore(), 'ask').mockImplementation(() => {})
  await button(view, 'Source game')!.trigger('click')
  await flushPromises()
  expect(ask).toHaveBeenCalledWith(
    expect.objectContaining({ title: 'Replace the unsaved analysis?' }),
  )
  expect(analysis.root.fen).toBe(fen)
  view.unmount()
})

it('says why the source game cannot open without a connection', async () => {
  const exportGame = vi.fn(async () => {
    throw new Error('fetch failed')
  })
  await board({ exportGame })
  const online = vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(false)
  window.dispatchEvent(new Event('offline'))
  usePuzzleAnalysis().open(puzzle)
  const view = card()
  await nextTick()
  await button(view, 'Source game')!.trigger('click')
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toBe(
    'Can’t reach Lichess. Connect to the internet to open this game.',
  )
  // The puzzle stays on the board.
  expect(useAnalysisStore().root.fen).toBe(fen)
  online.mockRestore()
  view.unmount()
})

it('marks the puzzle’s setup move before its first move', async () => {
  const { analysis } = await board()
  usePuzzleAnalysis().open(puzzle)
  const Board = defineComponent({ props: ['lastMove'], template: '<div />' })
  const stubs = Object.fromEntries(
    ['PageHeader', 'StudyBar', 'EvalBar', 'AnalysisLine', 'MoveNotes', 'AnalysisPuzzle'].map(
      (name) => [name, true],
    ),
  )
  const page = shallowMount(AnalysisPage, { global: { stubs: { ...stubs, ChessBoard: Board } } })
  await nextTick()
  expect(page.getComponent(Board).props('lastMove')).toEqual(['f6', 'e5'])
  analysis.forward()
  await nextTick()
  expect(page.getComponent(Board).props('lastMove')).toEqual(['d1', 'h5'])
  page.unmount()
})

// A puzzle's few moves make side-by-side accuracy and practice meaningless; its chart is not.
it('reviews a puzzle line with its chart, without the players’ figures', async () => {
  await board()
  usePuzzleAnalysis().open(puzzle)
  const key = reviewKey(fen, puzzle.solution)
  useReviewStore().reviews.set(key, {
    key,
    fen,
    moves: puzzle.solution,
    source: 'local',
    evals: [{ cp: 300 }, { cp: 320 }, { cp: 330 }, { cp: 900 }],
    depth: 18,
    complete: true,
    updatedAt: 0,
  })
  await flushPromises()
  const review = mount(AnalysisReview, {
    global: { stubs: { UButton: true, UTooltip: true, UProgress: true, EvalChart: true } },
  })
  expect(review.find('eval-chart-stub').exists()).toBe(true)
  expect(review.find('.review-table').exists()).toBe(false)
  review.unmount()
})
