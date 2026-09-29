<script setup lang="ts">
import { computed } from 'vue'
import { formatGameDate, gameResult } from '../utils/games'
import type { LichessGame } from '../../src/shared/types'

const props = defineProps<{ game: LichessGame; detailed?: boolean }>()
const emit = defineEmits<{ select: [] }>()
const result = computed(() => gameResult(props.game))
const badge = computed(
  () =>
    ({
      win: { label: 'Win', color: 'success' },
      loss: { label: 'Loss', color: 'error' },
      draw: { label: 'Draw', color: 'neutral' },
    })[result.value] as { label: string; color: 'success' | 'error' | 'neutral' },
)
const diff = computed(() => props.game.ratingDiff)
</script>

<template>
  <button
    type="button"
    class="game-row"
    :aria-label="`${badge.label} against ${game.opponent}, ${formatGameDate(game.createdAt)}. Open game`"
    @click="emit('select')"
  >
    <span
      ><UBadge :color="badge.color" variant="soft" class="w-11 justify-center">{{
        badge.label
      }}</UBadge></span
    >
    <span class="opponent">
      <strong>{{ game.opponent }}</strong>
      <span v-if="detailed" class="sub">
        {{ game.opening ?? game.account
        }}<template v-if="game.opponentRating"> · {{ game.opponentRating }}</template>
      </span>
    </span>
    <span class="opponent">
      <span class="capitalize-first">{{ game.speed }}</span>
      <span v-if="detailed" class="sub">{{ game.rated ? 'Rated' : 'Casual' }}</span>
    </span>
    <span v-if="detailed" class="rating-diff" :class="diff ? (diff > 0 ? 'up' : 'down') : 'muted'">
      {{ diff ? (diff > 0 ? `+${diff}` : diff) : '—' }}
    </span>
    <span class="muted">{{ formatGameDate(game.createdAt) }}</span>
  </button>
</template>
