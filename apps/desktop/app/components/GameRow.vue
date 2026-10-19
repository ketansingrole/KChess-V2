<script setup lang="ts">
import { computed } from 'vue'
import { formatGameDate, OUTCOME_BADGES } from '../utils/games'
import { gameResult } from '@kchess/rules/gameStatus'
import { judgmentCounts } from '../utils/review'
import type { LichessGame, ReviewSummary } from '@kchess/contracts/types'

const props = defineProps<{
  game: LichessGame
  detailed?: boolean
  review?: ReviewSummary
  /** Leave out the accuracy column, for lists that have no reviews to show. */
  hideAccuracy?: boolean
}>()
const emit = defineEmits<{ select: [] }>()
const badge = computed(() => OUTCOME_BADGES[gameResult(props.game)])
const diff = computed(() => props.game.ratingDiff)
/** The player's side of the game's review. */
const side = computed(() => props.review?.[props.game.color])
/** The player's worst kind of move in the game, counted (“2 blunders”). */
const worst = computed(() => (side.value ? judgmentCounts(side.value)[0] : undefined))
</script>

<template>
  <button
    type="button"
    class="game-row"
    :aria-label="`${badge.label} against ${game.opponent}, ${formatGameDate(game.createdAt)}. Open game`"
    @click="emit('select')"
  >
    <span><ResultBadge :badge="badge" class="w-11" /></span>
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
    <span v-if="detailed && !hideAccuracy" class="opponent tabular">
      <span :class="{ muted: !side }">{{
        side?.accuracy !== undefined ? `${side.accuracy}%` : side ? 'Reviewed' : '—'
      }}</span>
      <span v-if="worst" class="sub" :class="worst.judgment">{{ worst.text }}</span>
    </span>
    <span class="muted">{{ formatGameDate(game.createdAt) }}</span>
  </button>
</template>
