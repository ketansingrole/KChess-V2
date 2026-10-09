<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'

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
  }>(),
  { title: 'Moves', controls: true, selectable: true, flipLabel: 'Flip board' },
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
  { icon: 'i-lucide-skip-back', label: 'Go to first move', keys: ['↑'] },
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
        {{ Math.ceil(moves.length / 2) }} {{ Math.ceil(moves.length / 2) === 1 ? 'move' : 'moves' }}
      </span>
    </div>
    <div class="moves-wrap">
      <div ref="list" class="moves-list" role="group" :aria-label="title">
        <template v-for="(san, index) in props.moves" :key="index">
          <div v-if="index % 2 === 0" class="move-num" aria-hidden="true">{{ index / 2 + 1 }}</div>
          <button
            v-if="selectable"
            type="button"
            class="move-cell"
            :class="{ active: props.ply === index + 1 }"
            :aria-label="`Move ${Math.floor(index / 2) + 1}, ${index % 2 ? 'black' : 'white'}, ${san}`"
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
