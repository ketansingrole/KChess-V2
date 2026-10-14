<script setup lang="ts">
import { ref } from 'vue'
import type PuzzleBoard from './PuzzleBoard.vue'

/**
 * Voice moves for whichever puzzle board is showing, placed in that board's bottom row. It lives
 * with the page rather than the board: boards are replaced for every puzzle, and restarting voice
 * input would reload the speech model each time.
 */
defineProps<{
  board: InstanceType<typeof PuzzleBoard> | null
  /** Changes with every puzzle, so a pending spoken move never carries over. */
  contextKey: string
}>()
const enabled = ref(false)
</script>

<template>
  <div hidden>
    <Teleport :to="board?.controls ?? null" :disabled="!board?.controls">
      <VoiceMoveInput
        v-model:enabled="enabled"
        source="puzzles"
        :position="board?.position"
        :active="board?.playing ?? false"
        :context-key="contextKey"
        hint="Say “E four” or “Knight F three”."
        :play="(uci: string) => board?.playMove(uci) ?? false"
      />
    </Teleport>
  </div>
</template>
