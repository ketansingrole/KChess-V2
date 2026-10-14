<script setup lang="ts">
import { computed } from 'vue'
import type { Color } from '@lichess-org/chessground/types'
import { materialDiff, type MaterialRole } from '@kchess/rules/material'
import { useKChessStore } from '../stores/kchess'
import { pieceUrl } from '../utils/pieces'

/** The pieces one side has won over the other, and its lead in pawns, as Lichess shows them. */
const props = defineProps<{ fen: string; color: Color }>()

const LETTER = { king: 'K', queen: 'Q', rook: 'R', bishop: 'B', knight: 'N', pawn: 'P' } as const
const store = useKChessStore()
const diff = computed(() => materialDiff(props.fen))
/** Same pieces sit together, so "two pawns" reads as one group. */
const groups = computed(() => {
  const groups: { role: MaterialRole; count: number }[] = []
  for (const role of diff.value[props.color]) {
    const last = groups.at(-1)
    if (last?.role === role) last.count++
    else groups.push({ role, count: 1 })
  }
  return groups
})
const lead = computed(() => (props.color === 'white' ? 1 : -1) * diff.value.score)
const pieceSet = computed(() => store.settings?.pieceSet ?? 'cburnett')
const label = computed(() => {
  const pieces = groups.value.map(({ role, count }) => (count > 1 ? `${count} ${role}s` : role))
  return [lead.value > 0 ? `Up ${lead.value}` : '', pieces.join(', ')].filter(Boolean).join(': ')
})
</script>

<template>
  <span v-if="groups.length || lead > 0" class="material" role="img" :aria-label="label">
    <span v-for="group in groups" :key="group.role" class="material-group">
      <i
        v-for="n in group.count"
        :key="n"
        :style="{ '--piece': `url(&quot;${pieceUrl(pieceSet, 'b', LETTER[group.role])}&quot;)` }"
      />
    </span>
    <span v-if="lead > 0" class="material-lead tabular">+{{ lead }}</span>
  </span>
</template>
