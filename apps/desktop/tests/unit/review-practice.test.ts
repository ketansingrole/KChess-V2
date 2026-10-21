import { expect, it } from 'vitest'
import { defineComponent, nextTick } from 'vue'
import { mount, shallowMount } from '@vue/test-utils'
import AnalysisPage from '../../app/pages/analysis.vue'
import ReviewPractice from '../../app/components/ReviewPractice.vue'
import { useReviewPractice } from '../../app/stores/reviewPractice'
import { BLUNDER_PLY, key, moves, reviewedGame } from './reviewedGame'

it('practises a reviewed position on the board and schedules the answer', async () => {
  const { analysis, practice: saved } = await reviewedGame()
  await saved.add(analysis.review!, 'black')
  const practice = useReviewPractice()
  practice.start(key)
  expect(practice.status).toBe('find')
  expect(practice.played).toEqual({
    uci: 'g7g6',
    label: '4… g6',
    judgment: 'blunder',
    color: 'black',
  })
  expect(practice.playedArrow).toBe('g7g6')
  expect(analysis.reviewPly).toBe(BLUNDER_PLY)
  expect(analysis.orientation).toBe('black')
  expect(practice.hidesEngine).toBe(true)
  // The move from the game is wrong again: nothing is added and the board stays.
  expect(practice.attempt('g7g6')).toBe('wrong')
  expect(practice.tried).toBe('g6')
  expect(analysis.reviewPly).toBe(BLUNDER_PLY)
  practice.hint()
  expect(practice.hintSquare).toBe('e8')
  // Elsewhere in the game a move is an ordinary analysis move.
  analysis.goToPly(2)
  expect(practice.attempt('g1f3')).toBeNull()
  practice.show()
  expect(practice.attempt('e8e7')).toBe('solved')
  expect(practice.answerSan).toBe('Ke7')
  // The answer is kept beside the game's move, which stays the main line.
  expect(analysis.path).toBe([...moves.slice(0, BLUNDER_PLY), 'e8e7'].join(' '))
  expect(analysis.mainline).toEqual(moves)
  expect(saved.answered).toHaveBeenCalledWith(`${key}:${BLUNDER_PLY}`, false)
  expect(practice.hidesEngine).toBe(false)
  practice.next()
  expect(practice.status).toBe('done')
  expect(practice.found).toEqual([false])
  practice.again()
  expect(practice.status).toBe('find')
  expect(practice.attempt('e8e7')).toBe('solved')
  expect(saved.answered).toHaveBeenLastCalledWith(`${key}:${BLUNDER_PLY}`, true)
  practice.stop()
  expect(practice.active).toBe(false)
  expect(analysis.orientation).toBe('white')
})

it('replaces the engine with the practice card and keeps wrong answers off the board', async () => {
  const { analysis, practice: saved } = await reviewedGame()
  analysis.engineOn = true
  await saved.add(analysis.review!, 'black')
  const Board = defineComponent({
    props: ['shapes', 'resetKey', 'fen'],
    emits: ['move'],
    template: '<div />',
  })
  const stubs = Object.fromEntries(
    [
      'PageHeader',
      'StudyBar',
      'EvalBar',
      'UButton',
      'UIcon',
      'UContextMenu',
      'UDropdownMenu',
      'UFormField',
      'UModal',
      'UPopover',
      'USelect',
      'USwitch',
      'UTooltip',
      'VoiceInput',
      'AnalysisLine',
      'AnalysisReview',
      'ExportGame',
      'MoveNotes',
      'PositionExplorer',
      'EngineLines',
      'ReviewPractice',
    ].map((name) => [name, true]),
  )
  const page = shallowMount(AnalysisPage, {
    global: { stubs: { ...stubs, ChessBoard: Board } },
  })
  const practice = useReviewPractice()
  practice.start(key)
  await nextTick()
  expect(page.find('review-practice-stub').exists()).toBe(true)
  expect(page.find('.ceval').exists()).toBe(false)
  expect(page.find('eval-bar-stub').exists()).toBe(false)
  const board = page.getComponent(Board)
  // The game's move is drawn, to find a better one; the engine's arrows are not.
  const played = { orig: 'g7', dest: 'g6', brush: 'red' }
  expect(board.props('shapes')).toEqual([played])
  const before = board.props('resetKey')
  board.vm.$emit('move', 'g7g6')
  await nextTick()
  expect(practice.status).toBe('wrong')
  expect(board.props('resetKey')).toBe(before + 1)
  practice.hint()
  await nextTick()
  expect(board.props('shapes')).toEqual([played, { orig: 'e8', brush: 'green' }])
  board.vm.$emit('move', 'e8e7')
  await nextTick()
  expect(practice.status).toBe('solved')
  expect(board.props('fen')).toBe(analysis.node.fen)
  practice.stop()
  await nextTick()
  expect(page.find('.ceval').exists()).toBe(true)
  expect(page.find('eval-bar-stub').exists()).toBe(true)
  page.unmount()
})

it('names the players beside the move to improve on', async () => {
  const { analysis, practice: saved } = await reviewedGame()
  await saved.add(analysis.review!, 'black')
  analysis.root.headers = { White: 'Alice', Black: 'Bob' }
  useReviewPractice().start(key)
  const card = mount(ReviewPractice, {
    global: { stubs: { UButton: true, UTooltip: true, UIcon: true, EvalChart: true } },
  })
  await nextTick()
  const players = () =>
    card.findAll('.practice-player').map((player) => ({
      side: player.get('.sr-only').text(),
      name: player.get('.practice-name').text(),
      you: player.find('.practice-you').exists(),
    }))
  expect(players()).toEqual([
    { side: 'White:', name: 'Alice', you: false },
    { side: 'Black:', name: 'Bob', you: false },
  ])
  expect(card.get('.practice-played').text()).toBe('Bob played 4… g6??, a blunder.')
  // Your own game: you are marked, and it is your move to improve on.
  analysis.origin = { player: 'black' }
  await nextTick()
  expect(players()[1]).toEqual({ side: 'Black:', name: 'Bob', you: true })
  expect(card.get('.practice-played').text()).toBe('You played 4… g6??, a blunder.')
  card.unmount()
})
