<script setup lang="ts">
import { computed } from 'vue'
import type { Color } from '@lichess-org/chessground/types'
import type { EngineLine } from '../../../../core/src/contracts/types'
import { formatEval, winningChances } from '../../../../core/src/domain/analysisTree'

/** Lichess-style vertical evaluation bar: White's share fills from White's side of the board. */
const props = defineProps<{ line?: EngineLine; orientation: Color; result?: string }>()
const known = computed(() => Boolean(props.line || (props.result && props.result !== '*')))

const white = computed(() => {
  if (props.result === '1-0') return 100
  if (props.result === '0-1') return 0
  if (props.result) return 50
  return 50 + 50 * winningChances(props.line)
})
const label = computed(() => props.result || formatEval(props.line))
</script>

<template>
  <div
    class="eval-bar"
    :class="{ flipped: orientation === 'black', pending: !known }"
    role="meter"
    aria-label="Evaluation"
    :aria-valuenow="known ? Math.round(white) : undefined"
    aria-valuemin="0"
    aria-valuemax="100"
    :aria-valuetext="label"
    :title="label"
  >
    <div v-if="known" class="eval-white" :style="{ height: `${white}%` }" />
    <div v-if="known" class="eval-mid" />
  </div>
</template>

<style scoped>
.eval-bar {
  position: relative;
  display: flex;
  flex-direction: column-reverse;
  width: 100%;
  height: 100%;
  overflow: hidden;
  border-radius: 4px;
  background: #403d39;
}
.eval-bar.flipped {
  flex-direction: column;
}
.eval-bar.pending {
  background: repeating-linear-gradient(135deg, #403d39 0 4px, #68645e 4px 8px);
}
.eval-white {
  background: #f2f0ea;
  transition: height 0.35s ease;
}
.eval-mid {
  position: absolute;
  inset-inline: 0;
  top: 50%;
  height: 2px;
  margin-top: -1px;
  background: rgb(214 79 0 / 55%);
}
</style>
