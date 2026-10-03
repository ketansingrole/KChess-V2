<script setup lang="ts">
import { ref, watch } from 'vue'
import type { PositionLookup, PositionLookupKind } from '../../src/shared/types'
import { useAnalysisStore } from '../stores/analysis'
const analysis = useAnalysisStore()
const result = ref<PositionLookup | null>(null)
const error = ref('')
const busy = ref(false)
let epoch = 0
watch(
  () => analysis.node.fen,
  () => {
    epoch++
    result.value = null
    error.value = ''
    busy.value = false
  },
)
async function lookup(kind: PositionLookupKind): Promise<void> {
  const request = ++epoch
  const fen = analysis.node.fen
  busy.value = true
  error.value = ''
  try {
    const data = await window.kchess.positionLookup(kind, fen)
    if (request === epoch) result.value = data
  } catch (cause) {
    if (request === epoch)
      error.value = cause instanceof Error ? cause.message : 'Lookup failed. Retry.'
  } finally {
    if (request === epoch) busy.value = false
  }
}
function category(value: string | undefined, opponent = false): string {
  if (!value) return ''
  if (opponent)
    value = value
      .replace(/win|loss/g, (part) => (part === 'win' ? 'loss' : 'win'))
      .replace('blessed-win', 'cursed-win')
      .replace('cursed-loss', 'blessed-loss')
  return value.replace(/-/g, ' ')
}
</script>
<template>
  <details class="panel-divider py-3">
    <summary class="cursor-pointer font-semibold">Opening explorer and tablebases</summary>
    <p class="mt-2 text-xs text-muted">
      Lookups send this position to Lichess. Opening lookup uses a connected account. Saved results
      remain available offline. Statistics describe games, not move quality.
    </p>
    <div class="mt-2 flex gap-2">
      <UButton
        size="xs"
        variant="outline"
        color="neutral"
        :disabled="busy"
        @click="lookup('opening')"
        >Opening games</UButton
      >
      <UButton
        size="xs"
        variant="outline"
        color="neutral"
        :disabled="busy"
        @click="lookup('tablebase')"
        >Endgame tablebase</UButton
      >
    </div>
    <p v-if="busy" role="status" class="mt-2 text-sm">Loading position lookup…</p>
    <p v-if="error" role="alert" class="mt-2 text-sm">{{ error }}</p>
    <div v-if="result" class="mt-2 text-sm">
      <p>
        {{
          result.opening ||
          (result.kind === 'opening'
            ? 'Lichess rated games · Blitz, Rapid, Classical'
            : 'Tablebase · ' + category(result.category) + ' for the side to move')
        }}
      </p>
      <p v-if="result.kind === 'tablebase'" class="text-xs text-muted">
        DTZ {{ result.dtz ?? 'unknown' }} plies to a pawn move or capture; DTZ is not distance to
        mate. Cursed wins/blessed losses account for the fifty-move rule.
      </p>
      <p v-if="result.cached" class="text-xs text-muted">
        {{ result.message || 'Saved lookup' }} ·
        {{ new Date(result.fetchedAt).toLocaleDateString() }}
      </p>
      <p v-if="!result.moves.length" class="mt-2">No moves found for this position.</p>
      <ul class="mt-2 space-y-1">
        <li v-for="move in result.moves" :key="move.uci" class="flex items-center gap-2">
          <UButton size="xs" variant="link" @click="analysis.play(move.uci)">{{
            move.san
          }}</UButton>
          <span v-if="result.kind === 'opening'" class="text-xs"
            >White {{ move.white }} · Draw {{ move.draws }} · Black {{ move.black }}</span
          >
          <span v-else class="text-xs"
            >{{ category(move.category, true) }} for the current player</span
          >
        </li>
      </ul>
    </div>
  </details>
</template>
