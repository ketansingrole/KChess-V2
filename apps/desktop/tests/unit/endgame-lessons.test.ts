import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { LESSON_COURSES } from '@kchess/rules/lessons'
import type { RunInput, RunSummary } from '@kchess/contracts/types'
import { useKChessStore } from '../../app/stores/kchess'
import EndgameLesson from '../../app/components/EndgameLesson.vue'
import EndgameTraining from '../../app/components/EndgameTraining.vue'
import { ChessBoardStub, componentStubs, desktop } from './fixtures'

const summary = (best: Record<string, number>): RunSummary => ({
  kind: 'lesson',
  best: Object.fromEntries(Object.entries(best).map(([id, score]) => [id, { score, playedAt: 1 }])),
  recent: [],
  total: Object.keys(best).length,
})
const button = (wrapper: VueWrapper, text: string) =>
  wrapper.findAll('button').find((entry) => entry.text().includes(text))!

describe('endgame lessons', () => {
  it('saves a chapter as a lesson run once its last step is solved', async () => {
    const saveRun = vi.fn(async (run: RunInput) => ({
      summary: summary({ [run.variant]: run.score }),
      isBest: true,
    }))
    desktop({ saveRun })
    await useKChessStore().init()
    const course = LESSON_COURSES.find((entry) => entry.id === 'checkmate-patterns-2')!
    const wrapper = mount(EndgameLesson, {
      props: { course, summary: null },
      global: { stubs: componentStubs },
    })
    await button(wrapper, 'Balestra mate').trigger('click')
    await button(wrapper, 'Next').trigger('click')
    expect(wrapper.text()).toContain('Your turn')

    wrapper.findComponent(ChessBoardStub).vm.$emit('move', 'd3c4')
    await flushPromises()
    expect(wrapper.text()).toContain('Not the move — try again')
    expect(saveRun).not.toHaveBeenCalled()

    wrapper.findComponent(ChessBoardStub).vm.$emit('move', 'd3b5')
    await flushPromises()
    expect(wrapper.text()).toContain('Checkmate!')
    expect(saveRun).toHaveBeenCalledTimes(1)
    expect(saveRun.mock.calls[0]?.[0]).toMatchObject({
      kind: 'lesson',
      variant: 'cp2-balestra',
      score: 1,
    })
    expect(wrapper.emitted('saved')?.[0]?.[0]).toMatchObject({ best: { 'cp2-balestra': {} } })
    wrapper.unmount()
  })

  // Lessons showed the blindfold toggle but never passed the setting to their board.
  it('hides the pieces while a step is played blindfolded, not while reading or once it ends', async () => {
    desktop()
    const store = useKChessStore()
    await store.init()
    store.settings!.blindfold = true
    const course = LESSON_COURSES.find((entry) => entry.id === 'checkmate-patterns-2')!
    const wrapper = mount(EndgameLesson, {
      props: { course, summary: null },
      global: { stubs: componentStubs },
    })
    const board = () => wrapper.findComponent(ChessBoardStub)
    await button(wrapper, 'Balestra mate').trigger('click')
    expect(board().props('blindfold')).toBe(false)
    await button(wrapper, 'Next').trigger('click')
    expect(board().props('blindfold')).toBe(true)
    board().vm.$emit('move', 'd3b5')
    await flushPromises()
    expect(wrapper.text()).toContain('Checkmate!')
    expect(board().props('blindfold')).toBe(false)
    wrapper.unmount()
  })

  it('opens a course at its first unfinished chapter', async () => {
    desktop()
    await useKChessStore().init()
    const course = LESSON_COURSES.find((entry) => entry.id === 'checkmate-patterns-1')!
    const wrapper = mount(EndgameLesson, {
      props: { course, summary: summary({ 'cp1-back-rank': 1 }) },
      global: { stubs: componentStubs },
    })
    expect(wrapper.find('.side-panel h2').text()).toBe('Hook mate')
    wrapper.unmount()
  })

  it('shows each course with its finished chapters from the core', async () => {
    const runSummary = vi.fn(async (kind: string) =>
      kind === 'lesson'
        ? summary({ 'cp2-balestra': 1, 'cp2-arabian': 1 })
        : { ...summary({}), kind: kind as 'endgame' },
    )
    desktop({ runSummary })
    localStorage.clear()
    await useKChessStore().init()
    const wrapper = mount(EndgameTraining, { global: { stubs: componentStubs } })
    await flushPromises()
    expect(runSummary).toHaveBeenCalledWith('lesson')
    expect(button(wrapper, 'Checkmate Patterns II').text()).toContain('2 / 9')
    expect(button(wrapper, 'Piece Checkmates I').text()).toContain('0 / 6')
    expect(wrapper.text()).toContain('Rook endgames')
    await button(wrapper, 'Checkmate Patterns II').trigger('click')
    expect(wrapper.findComponent(EndgameLesson).exists()).toBe(true)
    wrapper.unmount()
  })
})
