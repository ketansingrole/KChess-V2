import { expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import type { MistakeGame } from '@kchess/rules/library'
import { INITIAL_FEN } from '@kchess/rules/position'
import MistakeTrainer from '../../app/components/MistakeTrainer.vue'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useKChessStore } from '../../app/stores/kchess'
import { useMistakeStore } from '../../app/stores/mistakes'
import { useReviewPractice } from '../../app/stores/reviewPractice'
import { desktop } from './fixtures'
import { BLUNDER_PLY, key, moves } from './reviewedGame'

const DAY = 86_400_000
const item = (id: string, judgment: string, dueAt: number, streak = 0, fen = INITIAL_FEN) => ({
  id,
  fen,
  solution: ['e2e4'],
  judgment,
  dueAt,
  streak,
  attempts: streak,
})
const lichessGame: MistakeGame = {
  key,
  fen: INITIAL_FEN,
  moves,
  playedAt: Date.UTC(2026, 9, 20),
  white: 'Rival',
  black: 'Alice',
  player: 'black',
  gameId: 'Game0001',
  account: 'Alice',
}
const analysisGame: MistakeGame = {
  key: 'other',
  fen: INITIAL_FEN,
  moves: ['e2e4'],
  playedAt: Date.UTC(2026, 9, 1),
}

async function trainer(games: MistakeGame[]) {
  desktop({
    reviewGet: async () => null,
    onReviewUpdate: () => () => {},
    onReviewStatus: () => () => {},
    onAnalysis: () => () => {},
    startAnalysis: async () => 0,
    stopAnalysis: async () => {},
    mistakeGames: async () => games,
  })
  const app = useKChessStore()
  await app.init()
  const selectPage = vi.spyOn(app, 'selectPage').mockImplementation(() => {})
  const now = Date.now()
  useMistakeStore().items = [
    // Black to move before each of Black's mistakes (2… f6 and 4… g6).
    item(
      `${key}:${BLUNDER_PLY}`,
      'blunder',
      now - 1,
      0,
      'rnbqkbnr/pppp2pp/8/4p2Q/4P3/8/PPPP1PPP/RNB1KB1R b KQkq - 1 4',
    ),
    item(
      `${key}:3`,
      'mistake',
      now + DAY,
      3,
      'rnbqkbnr/pppp1ppp/8/4p3/4P3/5N2/PPPP1PPP/RNBQKB1R b KQkq - 1 2',
    ),
    item('other:0', 'inaccuracy', now + DAY, 1),
  ]
  const view = mount(MistakeTrainer, {
    global: {
      stubs: {
        UButton: {
          emits: ['click'],
          template: '<button @click="$emit(\'click\')"><slot /></button>',
        },
        UEmpty: true,
        PuzzleBoard: true,
      },
      directives: { boardFit: {} },
    },
  })
  await flushPromises()
  return { view, selectPage }
}

it('sums up the queue and lists the games the positions come from', async () => {
  const { view } = await trainer([lichessGame, analysisGame])
  const stats = view.findAll('.mistake-stats dd').map((dd) => dd.text())
  expect(stats).toEqual(['1', '2', '1'])
  const rows = view.findAll('.mistake-games li')
  expect(rows.map((row) => row.get('.game-title').text())).toEqual([
    'vs Rival',
    'Analysis board game',
  ])
  expect(rows[0]!.get('.game-sub').text()).toMatch(/· 2 positions · 1 due$/)
  expect(rows[0]!.findAll('.game-counts span').map((span) => span.text())).toEqual([
    '1 blunder',
    '1 mistake',
  ])
  expect(rows[1]!.get('.game-sub').text()).toMatch(/· 1 position$/)
  view.unmount()
})

it('shows the most recent games first and the rest on request', async () => {
  const many = Array.from({ length: 10 }, (_, n) => ({ ...analysisGame, key: `g${n}` }))
  const { view } = await trainer(many)
  expect(view.findAll('.mistake-games li')).toHaveLength(8)
  await view
    .findAll('button')
    .find((button) => button.text() === 'Show all 10 games')!
    .trigger('click')
  expect(view.findAll('.mistake-games li')).toHaveLength(10)
  view.unmount()
})

it('practises a game on the analysis board, opening it unless it is already there', async () => {
  const { view, selectPage } = await trainer([lichessGame])
  const practiseButton = () =>
    view.findAll('button').find((button) => button.text() === 'Practice')!
  await practiseButton().trigger('click')
  const analysis = useAnalysisStore()
  const practice = useReviewPractice()
  expect(selectPage).toHaveBeenCalledWith('analysis')
  expect(analysis.mainline).toEqual(moves)
  expect(analysis.origin).toMatchObject({ white: 'Rival', black: 'Alice', player: 'black' })
  // The first of the game's positions, in move order, from your side.
  expect(practice.key).toBe(key)
  expect(practice.current?.ply).toBe(3)
  expect(analysis.path).toBe(moves.slice(0, 3).join(' '))
  expect(analysis.orientation).toBe('black')
  expect(analysis.tool).toBe('moves')
  // Already on the board: kept with its variations, and nothing to confirm.
  analysis.play('d7d5')
  const ask = vi.spyOn(useKChessStore(), 'ask')
  practice.stop()
  await practiseButton().trigger('click')
  expect(ask).not.toHaveBeenCalled()
  expect(analysis.root.children[0]!.children[0]!.children[0]!.children).toHaveLength(2)
  expect(practice.active).toBe(true)
  view.unmount()
})
