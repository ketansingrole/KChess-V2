import { afterEach, expect, it, vi } from 'vitest'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { defineComponent, nextTick } from 'vue'
import OpeningTrainer from '../../app/components/OpeningTrainer.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { useStudyStore } from '../../app/stores/studies'
import { REPERTOIRE_STUDY_KEY } from '../../app/utils/storageKeys'
import { desktop, componentStubs } from './fixtures'

const BoardStub = defineComponent({
  name: 'ChessBoard',
  props: ['fen'],
  emits: ['move'],
  template: '<div data-board />',
})
const SelectStub = defineComponent({
  name: 'USelect',
  props: ['modelValue', 'items'],
  emits: ['update:modelValue'],
  template: '<div data-select />',
})
const stubs = {
  ...componentStubs,
  ChessBoard: BoardStub,
  USelect: SelectStub,
  UFormField: { props: ['label'], template: '<div :data-label="label"><slot /></div>' },
  UTabs: true,
  BoardViewToggles: true,
}

afterEach(() => vi.restoreAllMocks())

async function trainer(chapters: { name: string; pgn: string }[]) {
  desktop()
  await useKChessStore().init()
  const id = await useStudyStore().saveChapters('Repertoire', chapters)
  localStorage.setItem(REPERTOIRE_STUDY_KEY, id)
  const wrapper = mount(OpeningTrainer, { global: { stubs } })
  await flushPromises()
  const chapterSelect = () => wrapper.find('[data-label="Chapter"]').findComponent(SelectStub)
  return { wrapper, chapterSelect, study: useStudyStore().items[0]! }
}
/** The feedback banner's two lines. */
const feedback = (wrapper: VueWrapper) =>
  wrapper.findAll('.trainer-feedback strong, .trainer-feedback .detail').map((line) => line.text())
const play = async (wrapper: VueWrapper, uci: string) => {
  wrapper.findComponent(BoardStub).vm.$emit('move', uci)
  await nextTick()
}

// The trainer parsed only the study's first chapter, so a d4 chapter could never be drilled.
it('drills every chapter of a repertoire, together or one at a time', async () => {
  const { wrapper, chapterSelect, study } = await trainer([
    { name: 'Kings pawn', pgn: '1. e4 e5 *' },
    { name: 'Queens pawn', pgn: '1. d4 d5 *' },
  ])
  expect(chapterSelect().props('items')).toEqual([
    { label: 'All chapters', value: 'all' },
    { label: '1. Kings pawn', value: study.chapters[0]!.id },
    { label: '2. Queens pawn', value: study.chapters[1]!.id },
  ])
  await play(wrapper, 'd2d4')
  expect(wrapper.text()).toContain('d4 ✓')

  chapterSelect().vm.$emit('update:modelValue', study.chapters[1]!.id)
  await flushPromises()
  await play(wrapper, 'e2e4')
  expect(feedback(wrapper)).toEqual(['Not in your repertoire', 'Expected d4.'])
  await play(wrapper, 'd2d4')
  expect(wrapper.text()).toContain('d4 ✓')

  chapterSelect().vm.$emit('update:modelValue', study.chapters[0]!.id)
  await flushPromises()
  await play(wrapper, 'd2d4')
  expect(feedback(wrapper)).toEqual(['Not in your repertoire', 'Expected e4.'])
  wrapper.unmount()
})

it('keeps chapters from different starting positions apart', async () => {
  vi.spyOn(Math, 'random').mockReturnValue(0.99)
  const { wrapper } = await trainer([
    { name: 'Main', pgn: '1. e4 e5 *' },
    {
      name: 'Endgame',
      pgn: '[FEN "4k3/8/8/8/8/8/4P3/4K3 w - - 0 1"]\n[SetUp "1"]\n\n1. e4 *',
    },
  ])
  // The last starting position is drawn: its own line, not one merged into the other chapter.
  expect(wrapper.findComponent(BoardStub).props('fen')).toContain('4k3/8/8/8/8/8/4P3/4K3')
  await play(wrapper, 'e2e4')
  expect(wrapper.text()).toContain('correct 1 · wrong 0')
  wrapper.unmount()
})

// Feedback of one line and of two, and a long study note inside the banner, moved the controls
// below it on every move.
it('keeps the feedback steady and the study note below the controls', async () => {
  vi.spyOn(Math, 'random').mockReturnValue(0)
  const { wrapper } = await trainer([
    { name: 'Main', pgn: '1. e4 { The king pawn, for open games. } e5 2. Nf3 Nc6 *' },
  ])
  // A steady banner keeps the room of two lines, whichever it says.
  expect(wrapper.get('.trainer-feedback').classes()).toContain('steady')
  expect(feedback(wrapper)).toEqual(['New line.'])
  await play(wrapper, 'e2e4')
  expect(feedback(wrapper)).toEqual(['e4 ✓'])
  const panel = wrapper.get('.side-panel').element
  const note = wrapper.get('.trainer-comment')
  expect(note.text()).toBe('The king pawn, for open games.')
  const actions = panel.querySelector('.panel-actions')!
  expect(
    actions.compareDocumentPosition(note.element) & Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy()
  expect(wrapper.find('.trainer-feedback').text()).not.toContain('king pawn')
  await play(wrapper, 'd2d4')
  expect(feedback(wrapper)).toEqual(['e4 ✓'])
  wrapper.unmount()
})
