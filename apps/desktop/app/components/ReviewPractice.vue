<script setup lang="ts">
import { computed } from 'vue'
import { useAnalysisStore } from '../stores/analysis'
import { useReviewPractice } from '../stores/reviewPractice'
import { MOVE_CLASSIFICATIONS } from '../utils/moveClassification'

/**
 * A reviewed game's mistakes, practised on the analysis board: what was played, the move to find,
 * and the game's chart, so each position keeps its place in the game.
 */
const analysis = useAnalysisStore()
const { origin, reviewChart, reviewPly } = storeToRefs(analysis)
const practice = useReviewPractice()
const { status, positions, index, played, tried, answerSan, atPosition, hinted, found } =
  storeToRefs(practice)

const verdict = computed(() => (played.value ? MOVE_CLASSIFICATIONS[played.value.judgment] : null))
const mover = computed(() => {
  const color = played.value?.color
  if (!color) return ''
  if (origin.value?.player === color) return 'You'
  return origin.value?.[color] ?? (color === 'white' ? 'White' : 'Black')
})
const foundCount = computed(() => found.value.filter(Boolean).length)
const last = computed(() => index.value + 1 >= positions.value.length)
</script>

<template>
  <section class="practice" aria-label="Practice this game's mistakes">
    <div class="practice-head">
      <strong>Practice</strong>
      <span v-if="status !== 'done'" class="practice-progress"
        >{{ index + 1 }} of {{ positions.length }}</span
      >
      <span class="flex-1" />
      <UTooltip text="Stop practice">
        <UButton
          size="xs"
          variant="ghost"
          color="neutral"
          icon="i-lucide-x"
          aria-label="Stop practice"
          @click="practice.stop"
        />
      </UTooltip>
    </div>

    <template v-if="status === 'done'">
      <p class="practice-status" role="status">
        You found {{ foundCount }} of {{ positions.length }}.
      </p>
      <div class="practice-actions">
        <UButton size="sm" @click="practice.again">Practice again</UButton>
        <UButton size="sm" variant="outline" color="neutral" @click="practice.stop">Done</UButton>
      </div>
    </template>
    <template v-else>
      <p v-if="played && verdict" class="practice-played">
        {{ mover }} played
        <strong :style="{ color: verdict.color }">{{ played.label }}{{ verdict.glyph }}</strong
        >, {{ played.judgment === 'inaccuracy' ? 'an' : 'a' }} {{ played.judgment }}.
      </p>
      <p class="practice-status" :class="status" role="status">
        <template v-if="status === 'find'">Find a better move.</template>
        <template v-else-if="status === 'wrong'"
          ><strong>{{ tried }}</strong> isn't the best move. Try again.</template
        >
        <template v-else-if="status === 'solved'"
          ><UIcon name="i-lucide-check" /> <strong>{{ answerSan }}</strong> is the best
          move.</template
        >
        <template v-else
          >The best move was <strong>{{ answerSan }}</strong
          >.</template
        >
      </p>
      <div class="practice-actions">
        <template v-if="status === 'find' || status === 'wrong'">
          <template v-if="atPosition">
            <UButton
              size="sm"
              :variant="hinted ? 'soft' : 'outline'"
              :color="hinted ? 'primary' : 'neutral'"
              :aria-pressed="hinted"
              @click="practice.hint"
              >Hint</UButton
            >
            <UButton size="sm" variant="outline" color="neutral" @click="practice.reveal"
              >Show solution</UButton
            >
            <UButton size="sm" variant="ghost" color="neutral" @click="practice.next">Skip</UButton>
          </template>
          <UButton v-else size="sm" @click="practice.show">Back to the position</UButton>
        </template>
        <UButton v-else size="sm" @click="practice.next">{{
          last ? 'Finish' : 'Next mistake'
        }}</UButton>
      </div>
    </template>

    <EvalChart
      v-if="reviewChart"
      :points="reviewChart.points"
      :judgments="reviewChart.judgments"
      :current="reviewPly"
      @select="analysis.goToPly"
    />
  </section>
</template>

<style scoped>
.practice {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.practice-head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 24px;
  font-size: 13px;
}
.practice-progress {
  color: var(--ui-text-muted);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.practice-played {
  margin: 0;
  color: var(--ui-text-muted);
  font-size: 13px;
}
.practice-status {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
}
.practice-status.wrong {
  color: var(--judgment-blunder);
}
.practice-status.solved {
  color: var(--ui-success);
}
.practice-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
</style>
