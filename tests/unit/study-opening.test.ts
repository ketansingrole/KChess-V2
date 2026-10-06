import { expect, it } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent } from 'vue'
import Studies from '../../app/pages/studies.vue'
import LichessStudies from '../../app/components/LichessStudies.vue'
import ConfirmDialog from '../../app/components/ConfirmDialog.vue'
import StudyChapters from '../../app/components/StudyChapters.vue'
import { useAnalysisStore } from '../../app/stores/analysis'
import { useStudyStore } from '../../app/stores/studies'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop, componentStubs } from './fixtures'

const Modal = defineComponent({
  props: ['open'],
  template: '<div v-if="open" role="dialog"><slot name="body" /><slot name="footer" /></div>',
})
const Input = defineComponent({
  props: ['modelValue'],
  emits: ['update:modelValue'],
  template: `<input :value="modelValue" @input="$emit('update:modelValue', $event.target.value)" />`,
})
const global = {
  components: { ConfirmDialog },
  stubs: {
    ...componentStubs,
    PageHeader: true,
    LichessStudies: true,
    UTabs: true,
    UInput: Input,
    UDropdownMenu: true,
    UModal: Modal,
  },
}
const button = (wrapper: ReturnType<typeof mount>, text: string) =>
  wrapper.findAll('button').find((b) => b.text() === text)!

it('confirms replacement before closing the dialog and opens the full local study', async () => {
  desktop()
  const library = useStudyStore()
  const id = library.saveChapters('Two chapters', [
    { name: 'King pawn', pgn: '1. e4 *' },
    { name: 'Queen pawn', pgn: '1. d4 *' },
  ])
  const analysis = useAnalysisStore()
  analysis.loadPgn('1. c4 *')
  const wrapper = mount(Studies, { global })
  await wrapper.get('button[aria-label="Open Two chapters"]').trigger('click')
  expect(wrapper.find('[role="dialog"]').exists()).toBe(true)
  await button(wrapper, 'Replace').trigger('click')
  expect(analysis.studyId).toBe(id)
  expect(analysis.study!.chapters).toHaveLength(2)
  expect(analysis.pgn()).toContain('e4')
  expect(wrapper.find('[role="dialog"]').exists()).toBe(false)
  wrapper.unmount()
})

it('opens all cloud chapters as a study after accepting replacement, rather than loading one unlinked PGN', async () => {
  desktop({
    lichessStudies: async () => [{ id: 'Study001', name: 'Cloud study', updatedAt: 0 }],
    lichessStudyChapters: async () => [
      { name: 'First', pgn: '1. e4 *' },
      { name: 'Second', pgn: '1. d4 *' },
    ],
  })
  await useKChessStore().init()
  const analysis = useAnalysisStore()
  analysis.loadPgn('1. c4 *')
  const wrapper = mount(LichessStudies, { global })
  await flushPromises()
  await wrapper.get('button[aria-label="Open Cloud study"]').trigger('click')
  await flushPromises()
  expect(wrapper.find('[role="dialog"]').exists()).toBe(true)
  expect(analysis.study).toBeUndefined()
  await button(wrapper, 'Replace').trigger('click')
  expect(analysis.study!.name).toBe('Cloud study')
  expect(analysis.study!.chapters.map((c) => c.name)).toEqual(['First', 'Second'])
  expect(analysis.study!.cloud!.id).toBe('Study001')
  wrapper.unmount()
})

it('cancels replacement without changing the analysis or downloading an offline copy', async () => {
  desktop()
  const id = useStudyStore().save('Saved', '1. e4 *')
  const analysis = useAnalysisStore()
  analysis.loadPgn('1. c4 *')
  const wrapper = mount(Studies, { global })
  await wrapper.get('button[aria-label="Open Saved"]').trigger('click')
  await button(wrapper, 'Cancel').trigger('click')
  expect(analysis.studyId).not.toBe(id)
  expect(analysis.pgn()).toContain('c4')
  wrapper.unmount()
})

const SelectMenu = defineComponent({
  props: ['items', 'modelValue'],
  emits: ['update:modelValue'],
  template: `<select :value="modelValue" @change="$emit('update:modelValue', $event.target.value)"><option v-for="item in items" :key="item.value" :value="item.value">{{item.label}}</option></select>`,
})
const Dropdown = defineComponent({
  props: ['items'],
  template: `<div><slot/><button v-for="item in items.flat()" :key="item.label" :disabled="item.disabled" @click="item.onSelect()">{{item.label}}</button></div>`,
})

it('selects chapters from a compact dropdown and adds, renames, duplicates, and deletes the active chapter', async () => {
  desktop()
  const library = useStudyStore()
  const id = library.saveChapters('Workspace', [
    { name: 'First', pgn: '1. e4 *' },
    { name: 'Second', pgn: '1. d4 {Notes} *' },
  ])
  const analysis = useAnalysisStore()
  analysis.openStudy(id)
  const wrapper = mount(StudyChapters, {
    global: {
      ...global,
      stubs: { ...global.stubs, USelectMenu: SelectMenu, UDropdownMenu: Dropdown },
    },
  })
  expect(wrapper.find('input[aria-label="Chapter name"]').exists()).toBe(false)
  expect(wrapper.find('nav').exists()).toBe(false)
  await wrapper.get('select[aria-label="Study chapter"]').setValue(analysis.study!.chapters[1]!.id)
  expect(analysis.pgn()).toContain('d4')
  await button(wrapper, 'Rename chapter').trigger('click')
  await wrapper.get('input[aria-label="Chapter name"]').setValue('Queen pawn')
  await wrapper.get('form').trigger('submit')
  expect(analysis.study!.chapters[1]!.name).toBe('Queen pawn')
  await button(wrapper, 'Duplicate chapter').trigger('click')
  expect(analysis.study!.chapters).toHaveLength(3)
  expect(analysis.study!.chapters.at(-1)!.name).toBe('Queen pawn (copy)')
  expect(analysis.pgn()).toContain('Notes')
  const duplicateId = analysis.studyChapterId
  await button(wrapper, 'Delete chapter').trigger('click')
  expect(wrapper.find('[role="dialog"]').exists()).toBe(true)
  await wrapper
    .findAll('button')
    .find((b) => b.text() === 'Delete chapter' && b.attributes('color') === 'error')!
    .trigger('click')
  expect(analysis.study!.chapters).toHaveLength(2)
  expect(analysis.study!.chapters.some((c) => c.id === duplicateId)).toBe(false)
  expect(analysis.studyChapterId).toBe(analysis.study!.chapters[1]!.id)
  await button(wrapper, 'Add chapter').trigger('click')
  await wrapper.get('input[aria-label="Chapter name"]').setValue('Third')
  await wrapper.get('form').trigger('submit')
  expect(analysis.studyId).toBe(id)
  expect(analysis.study!.chapters).toHaveLength(3)
  expect(analysis.study!.chapters.at(-1)!.name).toBe('Third')
  wrapper.unmount()
})
