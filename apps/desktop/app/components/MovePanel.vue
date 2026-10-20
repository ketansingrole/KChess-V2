<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'

const props = withDefaults(
  defineProps<{
    moves: readonly string[]
    ply: number
    title?: string
    emptyText?: string
    /** Show first/previous/next/last/flip controls. */
    controls?: boolean
    /** Let moves be clicked to jump to that position. */
    selectable?: boolean
    /** What the flip button does, for its tooltip and label. */
    flipLabel?: string
    /** The starting position, when it is not White's first move: it sets the move numbers. */
    fen?: string
  }>(),
  { title: 'Moves', controls: true, selectable: true, flipLabel: 'Flip board' },
)

/** Black to move at the start shifts every move one column, under the start's move number. */
const start = computed(() => {
  const fields = props.fen?.trim().split(/\s+/) ?? []
  const fullmove = Number(fields[5])
  return {
    shift: fields[1] === 'b' ? 1 : 0,
    fullmove: Number.isInteger(fullmove) && fullmove > 0 ? fullmove : 1,
  }
})
/** The move number and side of the move at `index`. */
function moveAt(index: number): { number: number; black: boolean } {
  const slot = index + start.value.shift
  return { number: start.value.fullmove + Math.floor(slot / 2), black: slot % 2 === 1 }
}

const moveCount = computed(() =>
  props.moves.length ? moveAt(props.moves.length - 1).number - start.value.fullmove + 1 : 0,
)

const emit = defineEmits<{
  select: [ply: number]
  flip: []
}>()

const list = ref<HTMLElement | null>(null)
watch(
  () => [props.ply, props.moves.length],
  async () => {
    await nextTick()
    const container = list.value
    if (!container) return
    const active = container.querySelector<HTMLElement>('.move-cell.active')
    // With no move selected (or a live game) follow the newest move.
    const target = active ?? container.querySelector<HTMLElement>('.move-cell:last-of-type')
    if (!target) return
    const top = target.offsetTop
    const bottom = top + target.offsetHeight
    if (top < container.scrollTop) container.scrollTop = top
    else if (bottom > container.scrollTop + container.clientHeight)
      container.scrollTop = bottom - container.clientHeight
  },
  { immediate: true },
)

const controlButtons = [
  { icon: 'i-lucide-skip-back', label: 'Go to start', keys: ['↑'] },
  { icon: 'i-lucide-chevron-left', label: 'Previous move', keys: ['←'] },
  { icon: 'i-lucide-chevron-right', label: 'Next move', keys: ['→'] },
  { icon: 'i-lucide-skip-forward', label: 'Go to last move', keys: ['↓'] },
] as const
</script>

<template>
  <div class="card side-panel">
    <slot name="top" />
    <div class="flex items-baseline justify-between gap-2">
      <h2 class="section-title">{{ title }}</h2>
      <span v-if="moves.length" class="muted text-xs tabular">
        {{ moveCount }} {{ moveCount === 1 ? 'move' : 'moves' }}
      </span>
    </div>
    <div class="moves-wrap">
      <div ref="list" class="moves-list" role="group" :aria-label="title">
        <template v-for="(san, index) in props.moves" :key="index">
          <div v-if="!moveAt(index).black || index === 0" class="move-num" aria-hidden="true">
            {{ moveAt(index).number }}
          </div>
          <span v-if="index === 0 && moveAt(0).black" class="move-cell muted" aria-hidden="true"
            >…</span
          >
          <button
            v-if="selectable"
            type="button"
            class="move-cell"
            :class="{ active: props.ply === index + 1 }"
            :aria-label="`Move ${moveAt(index).number}, ${moveAt(index).black ? 'black' : 'white'}, ${san}`"
            :aria-current="props.ply === index + 1 ? 'step' : undefined"
            @click="emit('select', index + 1)"
          >
            {{ san }}
          </button>
          <span v-else class="move-cell" :class="{ active: props.ply === index + 1 }">{{
            san
          }}</span>
        </template>
        <div v-if="!props.moves.length" class="moves-empty">
          {{ props.emptyText ?? 'No moves to review.' }}
        </div>
      </div>
    </div>
    <div v-if="controls" class="move-controls">
      <UTooltip
        v-for="(button, index) in controlButtons"
        :key="button.label"
        :text="button.label"
        :kbds="[...button.keys]"
      >
        <UButton
          size="sm"
          variant="ghost"
          color="neutral"
          :icon="button.icon"
          :aria-label="button.label"
          :disabled="index < 2 ? props.ply === 0 : props.ply === props.moves.length"
          @click="emit('select', [0, props.ply - 1, props.ply + 1, props.moves.length][index] ?? 0)"
        />
      </UTooltip>
      <UTooltip :text="flipLabel">
        <UButton
          size="sm"
          variant="ghost"
          color="neutral"
          icon="i-lucide-arrow-up-down"
          :aria-label="flipLabel"
          @click="emit('flip')"
        />
      </UTooltip>
    </div>
    <slot name="bottom" />
  </div>
</template>
