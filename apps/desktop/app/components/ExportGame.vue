<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { useOnline } from '@vueuse/core'
import type { DropdownMenuItem } from '@nuxt/ui'
import type { GameSetup } from '@kchess/rules/variant'
import { setupPgn } from '@kchess/rules/chess'

const props = defineProps<{
  setup: GameSetup
  moves: readonly string[]
  orientation: 'white' | 'black'
  white: string
  black: string
  /** The position shown, for the PNG; the last one by default. */
  ply?: number
  result?: string
  /** The game's id when it was played on Lichess, so its link needs no upload. */
  lichessId?: string
}>()
const store = useKChessStore()
const online = useOnline()
const toast = useToast()
const busy = ref(false)
const progress = ref('')
let exportController: AbortController | undefined
onBeforeUnmount(() => exportController?.abort())

const name = computed(() =>
  `${props.white} vs ${props.black} ${new Date().toISOString().slice(0, 10)}`
    .replace(/[^\w .()+-]/g, '')
    .slice(0, 80),
)
function pgnText(): string {
  return setupPgn(props.setup, props.moves, {
    White: props.white,
    Black: props.black,
    Result: props.result ?? '*',
    Date: new Date().toISOString().slice(0, 10).replace(/-/g, '.'),
  })
}
/** Uploads the game to Lichess (as the online account, if any) and copies its page's link. */
const uploading = ref(false)
async function upload(): Promise<void> {
  if (busy.value || uploading.value) return
  uploading.value = true
  try {
    const url = props.lichessId
      ? `https://lichess.org/${props.lichessId}`
      : (await window.kchess.importGame(store.activeOnlineAccount, pgnText())).url
    await navigator.clipboard.writeText(url)
    toast.add({ title: 'Copied the Lichess link', description: url, icon: 'i-lucide-check' })
  } catch (cause) {
    console.warn('[export-game] Upload failed:', cause)
    toast.add({
      title: 'Could not upload the game',
      description: cause instanceof Error ? cause.message : String(cause),
      color: 'error',
    })
  } finally {
    uploading.value = false
  }
}
async function save(kind: 'gif' | 'png' | 'pgn'): Promise<void> {
  if (busy.value) return
  busy.value = true
  const controller = new AbortController()
  exportController = controller
  progress.value = ''
  try {
    // Drawing a GIF takes a moment for long games; load the encoder only when asked.
    const { gameGif, positionPng } = await import('../utils/gameImage')
    const options = {
      setup: { ...props.setup },
      moves: [...props.moves],
      orientation: props.orientation,
      boardTheme: store.settings.boardTheme,
      pieceSet: store.settings.pieceSet,
      white: props.white,
      black: props.black,
    }
    const data =
      kind === 'gif'
        ? await gameGif(options, 80, {
            signal: controller.signal,
            progress: (done, total) => {
              progress.value = `${done}/${total}`
            },
          })
        : kind === 'png'
          ? await positionPng(options, props.ply ?? props.moves.length)
          : pgnText()
    controller.signal.throwIfAborted()
    if (await window.kchess.saveExport({ name: name.value, kind, data }))
      toast.add({ title: `Saved the ${kind.toUpperCase()}`, icon: 'i-lucide-check' })
  } catch (cause) {
    if (controller.signal.aborted) return
    console.warn('[export-game] Export failed:', cause)
    toast.add({
      title: 'Could not export the game',
      description: cause instanceof Error ? cause.message : String(cause),
      color: 'error',
    })
  } finally {
    busy.value = false
    progress.value = ''
    if (exportController === controller) exportController = undefined
  }
}
const items = computed<DropdownMenuItem[][]>(() => [
  [
    { label: 'Animated GIF of the game', icon: 'i-lucide-film', onSelect: () => void save('gif') },
    { label: 'PNG of this position', icon: 'i-lucide-image', onSelect: () => void save('png') },
    { label: 'PGN file', icon: 'i-lucide-file-text', onSelect: () => void save('pgn') },
  ],
  [
    {
      label: 'Lichess link',
      icon: 'i-lucide-link',
      disabled: !props.lichessId && (!online.value || !props.moves.length),
      onSelect: () => void upload(),
    },
  ],
])
</script>

<template>
  <span v-if="busy" class="inline-block w-28 shrink-0 text-xs tabular-nums" role="status">{{
    progress || 'Preparing export…'
  }}</span>
  <UButton v-if="busy" size="xs" variant="ghost" @click="exportController?.abort()"
    >Cancel export</UButton
  >
  <UDropdownMenu :items="items">
    <UButton
      size="xs"
      variant="ghost"
      color="neutral"
      icon="i-lucide-download"
      :loading="busy || uploading"
      :disabled="busy || uploading || (!moves.length && ply === undefined)"
      >Export</UButton
    >
  </UDropdownMenu>
</template>
