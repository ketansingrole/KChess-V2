<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { Color } from '@lichess-org/chessground/types'

const props = defineProps<{ color: Color }>()

/**
 * A player's clock, ticking on its own: the page around it never re-renders for time passing.
 * The display pauses while the tab is hidden (the game clock itself keeps running).
 */
const store = useKChessStore()
const text = ref<string | undefined>(store.computerClockText(props.color))
function refresh(): void {
  text.value = store.computerClockText(props.color)
}
let timer: ReturnType<typeof setInterval> | undefined
function start(): void {
  stop()
  timer = setInterval(refresh, 200)
}
function stop(): void {
  if (timer !== undefined) clearInterval(timer)
  timer = undefined
}
function onVisibility(): void {
  if (document.hidden) stop()
  else {
    refresh()
    start()
  }
}
watch(
  () => props.color,
  () => refresh(),
)
onMounted(() => {
  refresh()
  if (!document.hidden) start()
  document.addEventListener('visibilitychange', onVisibility)
})
onBeforeUnmount(() => {
  stop()
  document.removeEventListener('visibilitychange', onVisibility)
})
</script>

<template>
  <span v-if="text" class="player-clock" :aria-label="`${color} clock`">{{ text }}</span>
</template>
