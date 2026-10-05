<script setup lang="ts">
import { computed, ref } from 'vue'
import type { DropdownMenuItem } from '@nuxt/ui'
import type { GameSetup } from '../../src/shared/variant'
import { setupPgn } from '../utils/chess'

const props = defineProps<{
  setup: GameSetup
  moves: readonly string[]
  orientation: 'white' | 'black'
  white: string
  black: string
  /** The position shown, for the PNG; the last one by default. */
  ply?: number
  result?: string
}>()
const store = useKChessStore()
const toast = useToast()
const busy = ref(false)

const name = computed(() =>
  `${props.white} vs ${props.black} ${new Date().toISOString().slice(0, 10)}`
    .replace(/[^\w .()+-]/g, '')
    .slice(0, 80),
)
async function save(kind: 'gif' | 'png' | 'pgn'): Promise<void> {
  busy.value = true
  try {
    // Drawing a GIF takes a moment for long games; load the encoder only when asked.
    const { gameGif, positionPng } = await import('../utils/gameImage')
    const options = {
      setup: props.setup,
      moves: props.moves,
      orientation: props.orientation,
      boardTheme: store.settings.boardTheme,
      pieceSet: store.settings.pieceSet,
      white: props.white,
      black: props.black,
    }
    const data =
      kind === 'gif'
        ? await gameGif(options)
        : kind === 'png'
          ? await positionPng(options, props.ply ?? props.moves.length)
          : setupPgn(props.setup, props.moves, {
              White: props.white,
              Black: props.black,
              Result: props.result ?? '*',
              Date: new Date().toISOString().slice(0, 10).replace(/-/g, '.'),
            })
    if (await window.kchess.saveExport({ name: name.value, kind, data }))
      toast.add({ title: `Saved the ${kind.toUpperCase()}`, icon: 'i-lucide-check' })
  } catch (cause) {
    toast.add({
      title: 'Could not export the game',
      description: cause instanceof Error ? cause.message : String(cause),
      color: 'error',
    })
  } finally {
    busy.value = false
  }
}
const items = computed<DropdownMenuItem[][]>(() => [
  [
    { label: 'Animated GIF of the game', icon: 'i-lucide-film', onSelect: () => void save('gif') },
    { label: 'PNG of this position', icon: 'i-lucide-image', onSelect: () => void save('png') },
    { label: 'PGN file', icon: 'i-lucide-file-text', onSelect: () => void save('pgn') },
  ],
])
</script>

<template>
  <UDropdownMenu :items="items">
    <UButton
      size="xs"
      variant="ghost"
      color="neutral"
      icon="i-lucide-download"
      :loading="busy"
      :disabled="!moves.length && ply === undefined"
      >Export</UButton
    >
  </UDropdownMenu>
</template>
