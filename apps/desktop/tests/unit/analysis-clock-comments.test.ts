import { expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { desktop } from './fixtures'
import { useAnalysisStore } from '../../app/stores/analysis'
import { pathOf } from '@kchess/rules/analysisTree'
import AnalysisLine from '../../app/components/AnalysisLine.vue'
import MoveNotes from '../../app/components/MoveNotes.vue'

const stubs = { UTooltip: { template: '<slot />' }, UIcon: true, UKbd: true }

// Lichess exports put each move's clock in its comment; the board showed them as notes.
it('shows a game’s clock times as clocks, not comments, and keeps them through an edit', async () => {
  desktop({ reviewGet: async () => null, onReviewUpdate: () => () => {} })
  const analysis = useAnalysisStore()
  analysis.loadPgn(
    '1. d4 { [%clk 0:15:00] } d5 { Solid. [%clk 0:15:00] } 2. c4 { [%clk 0:15:06] } *',
  )
  analysis.path = pathOf(['d2d4', 'd7d5', 'c2c4'])
  await nextTick()

  const line = mount(AnalysisLine, {
    props: { parent: analysis.root, parentPath: '', current: analysis.path, depth: 0 },
  })
  expect(line.findAll('.tree-comment').map((comment) => comment.text())).toEqual(['Solid.'])
  expect(line.text()).not.toContain('%clk')

  const notes = mount(MoveNotes, { global: { stubs } })
  expect(notes.find('.notes-subject').text()).toContain('15:06')
  expect(notes.find('textarea').exists()).toBe(false)
  await notes.find('.notes-add').trigger('click')
  const field = notes.find('textarea')
  await field.setValue('Strong ')
  // The field keeps what was typed; the move keeps its clock.
  expect((field.element as HTMLTextAreaElement).value).toBe('Strong ')
  expect(analysis.node.comments).toEqual(['Strong  [%clk 0:15:06]'])
  expect(analysis.pgn()).toContain('[%clk 0:15:06]')
  await field.setValue('')
  expect(analysis.node.comments).toEqual(['[%clk 0:15:06]'])
})
