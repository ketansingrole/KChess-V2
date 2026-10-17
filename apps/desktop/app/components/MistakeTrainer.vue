<script setup lang="ts">
import { computed, ref } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import { useMistakeStore } from '../stores/mistakes'
import type { Puzzle } from '@kchess/contracts/types'
const app = useKChessStore()
const practice = useMistakeStore()
const current = ref<Puzzle | null>(null)
const now = ref(Date.now())
useIntervalFn(() => {
  now.value = Date.now()
}, 60_000)
const due = computed(() => practice.items.filter((item) => item.dueAt <= now.value))
const done = ref(false)
const board = ref<{
  hint: () => void
  hinted: boolean
  showSolution: () => Promise<void>
} | null>(null)
function next(all = false): void {
  const candidates = [...(all ? practice.items : due.value)].sort((a, b) => a.dueAt - b.dueAt)
  const item = candidates.find((item) => done.value || item.id !== current.value?.id)
  // The position on the board is the only one left: keep it rather than blank the board.
  if (!item && candidates.some((candidate) => candidate.id === current.value?.id)) return
  current.value = item ? practice.puzzle(item) : null
  done.value = false
}
</script>
<template>
  <!-- A position on the board: the puzzle pages' layout, so the board fits the window. -->
  <div v-if="current" v-board-fit class="play-layout puzzle-wide">
    <div>
      <PuzzleBoard
        ref="board"
        :key="current.id"
        :puzzle="current"
        @outcome="practice.answered(current!.id, $event)"
        @done="done = true"
      />
    </div>
    <div class="card side-panel puzzle-panel">
      <div>
        <h2 class="section-title">Practice your mistakes</h2>
        <p class="section-hint">{{ due.length }} due · {{ practice.items.length }} saved</p>
      </div>
      <p v-if="practice.error" role="alert">{{ practice.error }}</p>
      <p v-if="done" role="status">Position completed. Your next revisit has been scheduled.</p>
      <div v-else class="panel-actions flex gap-2">
        <UButton
          :variant="board?.hinted ? 'soft' : 'outline'"
          :color="board?.hinted ? 'primary' : 'neutral'"
          :aria-pressed="board?.hinted ?? false"
          @click="board?.hint()"
          >Hint: piece to move</UButton
        >
        <UButton variant="ghost" color="neutral" @click="board?.showSolution()"
          >Show solution</UButton
        >
      </div>
      <div class="panel-actions flex gap-2">
        <UButton :disabled="!due.length" @click="next()">Next due position</UButton>
        <UButton variant="outline" color="neutral" @click="next(true)">Review early</UButton>
      </div>
    </div>
  </div>
  <div v-else class="card">
    <h2 class="section-title">Practice your mistakes</h2>
    <p class="section-hint">{{ due.length }} due · {{ practice.items.length }} saved</p>
    <p v-if="practice.error" role="alert">{{ practice.error }}</p>
    <UEmpty
      v-if="!practice.items.length"
      variant="naked"
      icon="i-lucide-book-open"
      title="No mistakes yet"
      description="When you review one of your games, its mistakes are added here."
      :actions="[
        {
          label: 'Review a game',
          icon: 'i-lucide-history',
          onClick: () => app.selectPage('history'),
        },
      ]"
    />
    <template v-else>
      <p v-if="!due.length" class="section-hint mt-3" role="status">You're caught up.</p>
      <div class="mt-3 flex gap-2">
        <UButton :disabled="!due.length" @click="next()">Start practice</UButton>
        <UButton variant="outline" color="neutral" @click="next(true)">Review early</UButton>
      </div>
    </template>
  </div>
</template>
