import { afterEach, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent, ref } from 'vue'
import { useOpeningDrill, type OpeningChoice } from '../../app/composables/useOpeningDrill'
import type { DesktopApi } from '../../contracts/types'

afterEach(() => vi.restoreAllMocks())

const reply = (uci: string, games: number) => ({ uci, white: games, draws: 0, black: games })

function drill(positionLookup: DesktopApi['positionLookup']) {
  window.kchess = { ...window.kchess, positionLookup }
  const choice = ref<OpeningChoice | null>({ name: "King's Pawn", moves: ['e2e4'] })
  let result!: ReturnType<typeof useOpeningDrill>
  mount(
    defineComponent({
      setup() {
        result = useOpeningDrill(choice, ref('white'))
        return () => null
      },
    }),
  )
  return result
}

// A lookup that failed past the named line (a 429 cooldown) cut the drill short without a word.
it('says the drill was cut short when a later lookup fails', async () => {
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  let calls = 0
  const { tree, note, preparing } = drill(async () => {
    if (++calls === 1) return { moves: [reply('e7e5', 60), reply('c7c5', 40)] } as never
    throw new Error('Too many lookups. Try again in 30 seconds.')
  })
  await flushPromises()
  expect(calls).toBe(2)
  expect(preparing.value).toBe(false)
  expect(tree.value?.children[0]?.children.map((node) => node.uci)).toEqual(['e7e5', 'c7c5'])
  expect(note.value).toBe(
    'Some popular replies are missing: Too many lookups. Try again in 30 seconds.',
  )
})

it('drills only the named line when its own lookup fails', async () => {
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  const { tree, note } = drill(async () => {
    throw new Error('Position lookup failed. Check your connection and retry.')
  })
  await flushPromises()
  expect(tree.value?.children[0]?.children).toEqual([])
  expect(note.value).toBe(
    'Only the named line: Position lookup failed. Check your connection and retry.',
  )
})
