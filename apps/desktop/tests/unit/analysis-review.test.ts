import { expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import AnalysisReview from '../../app/components/AnalysisReview.vue'
import { useReviewPractice } from '../../app/stores/reviewPractice'
import { BLUNDER_PLY, key, moves, reviewedGame as reviewed } from './reviewedGame'

const mountReview = () =>
  mount(AnalysisReview, {
    global: {
      stubs: {
        UButton: {
          emits: ['click'],
          template: '<button @click="$emit(\'click\')"><slot /></button>',
        },
        UTooltip: true,
        UProgress: true,
        EvalChart: true,
      },
    },
  })
const practiceButton = (review: ReturnType<typeof mountReview>) =>
  review.findAll('button').find((button) => /^Practice/.test(button.text()))

it('compares both sides’ accuracy, labelled moves, centipawn loss and phases', async () => {
  const { analysis } = await reviewed()
  const review = mountReview()
  const rows = review.findAll('tbody th').map((th) => th.text())
  expect(rows).toEqual([
    'Accuracy',
    'Inaccuracies',
    'Mistakes',
    'Blunders',
    'Avg. centipawn loss',
    'Opening',
  ])
  expect(review.findAll('thead th.player').map((th) => th.text())).toEqual(['White', 'Black'])
  const result = analysis.gameAnalysis!
  const accuracy = review.findAll('tr.accuracy td').map((td) => td.text())
  expect(accuracy).toEqual([`${result.white.accuracy}%`, `${result.black.accuracy}%`])
  // Black's g6 walks into a lost position: its count jumps to that move.
  expect(result.black.blunder).toBe(1)
  await review.get('[aria-label="1 blunder by Black: go to the next"]').trigger('click')
  expect(analysis.path).toBe(moves.join(' '))
  await review.get('[aria-label="0 blunders by White: go to the next"]').trigger('click')
  expect(analysis.path).toBe(moves.join(' '))
  review.unmount()
})

it('offers one practice button: both sides of a game you did not play', async () => {
  const { analysis, practice } = await reviewed()
  const review = mountReview()
  expect(practiceButton(review)!.text()).toBe('Practice mistakes')
  await practiceButton(review)!.trigger('click')
  await flushPromises()
  // White made no labelled move, so only Black's are added.
  expect(practice.add).toHaveBeenCalledTimes(1)
  expect(practice.add).toHaveBeenCalledWith(expect.objectContaining({ key }), 'black')
  // Practice starts here, on the board, at the position before the mistake, beside the moves.
  expect(useReviewPractice().key).toBe(key)
  expect(analysis.reviewPly).toBe(BLUNDER_PLY)
  expect(analysis.orientation).toBe('black')
  expect(analysis.tool).toBe('moves')
  expect(practiceButton(review)).toBeUndefined()
  review.unmount()
})

it('practises your own mistakes, saved with the review, and never offers your opponent’s', async () => {
  const { practice } = await reviewed({ player: 'black' })
  expect(practice.add).toHaveBeenCalledWith(expect.objectContaining({ key }), 'black')
  const review = mountReview()
  expect(practiceButton(review)!.text()).toBe('Practice your mistakes')
  await practiceButton(review)!.trigger('click')
  await flushPromises()
  expect(practice.add).toHaveBeenCalledTimes(1)
  expect(useReviewPractice().active).toBe(true)
  review.unmount()
})

it('offers no practice when you made no labelled move', async () => {
  await reviewed({ player: 'white' })
  const review = mountReview()
  expect(practiceButton(review)).toBeUndefined()
  review.unmount()
})
