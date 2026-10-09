<script setup lang="ts">
import { computed } from 'vue'
import type { Puzzle } from '@kchess/core/contracts/types'
import { playerColor } from '@kchess/core/domain/puzzle'
import { themeName } from '../utils/puzzleThemes'

/** The words beside a puzzle board: what to do, how it went, and the buttons that help. */
const props = defineProps<{
  puzzle: Puzzle
  status: string
  /** The first verdict: solved cleanly (true), not (false), or still open (null). */
  outcome: boolean | null
  feedback: string
  waiting: boolean
  mistakes: number
}>()

const emit = defineEmits<{ hint: []; solution: [] }>()

const color = computed(() => playerColor(props.puzzle))
const finished = computed(() => props.status === 'solved' || props.status === 'revealed')
const banner = computed<{
  kind: '' | 'win' | 'loss' | 'alert'
  icon: string
  title: string
  detail?: string
}>(() => {
  if (props.status === 'revealed')
    return {
      kind: 'loss',
      icon: 'i-lucide-eye',
      title: 'Solution shown — not counted as solved',
    }
  if (props.status === 'solved')
    return props.outcome
      ? {
          kind: 'win',
          icon: 'i-lucide-trophy',
          title: 'Puzzle solved!',
        }
      : {
          kind: '',
          icon: 'i-lucide-check',
          title: 'Solved after a mistake — not counted',
        }
  if (props.status === 'failed')
    return {
      kind: 'loss',
      icon: 'i-lucide-x',
      title: 'Not the move — puzzle over',
    }
  if (props.feedback === 'wrong')
    return {
      kind: 'loss',
      icon: 'i-lucide-x',
      title: 'That’s not the move — try again',
    }
  return {
    kind: '',
    icon: props.waiting ? 'i-lucide-hourglass' : 'i-lucide-mouse-pointer-click',
    title: props.waiting ? 'Good move…' : `Find the best move for ${color.value}`,
  }
})
</script>

<template>
  <div class="status-banner" :class="banner.kind" role="status">
    <UIcon :name="banner.icon" class="status-icon" />
    <div>
      <strong>{{ banner.title }}</strong>
      <span v-if="banner.detail" class="detail">{{ banner.detail }}</span>
    </div>
  </div>
  <div class="panel-actions">
    <UButton
      variant="outline"
      color="neutral"
      icon="i-lucide-lightbulb"
      :disabled="status !== 'playing'"
      @click="emit('hint')"
      >Hint</UButton
    >
    <UButton
      variant="outline"
      color="neutral"
      icon="i-lucide-eye"
      :disabled="status !== 'playing'"
      @click="emit('solution')"
      >Solution</UButton
    >
    <slot name="actions" />
  </div>
  <dl v-if="finished || status === 'failed'" class="puzzle-facts">
    <div>
      <dt>Rating</dt>
      <dd class="tabular">{{ puzzle.rating }}</dd>
    </div>
    <div>
      <dt>Themes</dt>
      <dd class="theme-chips">
        <UBadge
          v-for="theme in puzzle.themes"
          :key="theme"
          color="neutral"
          variant="subtle"
          size="sm"
          >{{ themeName(theme) }}</UBadge
        >
      </dd>
    </div>
  </dl>
  <a
    v-if="finished || status === 'failed'"
    class="link text-xs"
    :href="`https://lichess.org/training/${puzzle.id}`"
    target="_blank"
    rel="noopener"
    >Open this puzzle on Lichess ↗</a
  >
</template>
