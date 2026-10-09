<script setup lang="ts">
import { computed, ref } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import { useMistakeStore } from '../stores/mistakes'
import type { Puzzle } from '@kchess/core/contracts/types'
const app = useKChessStore()
const practice = useMistakeStore()
const current = ref<Puzzle | null>(null)
const now = ref(Date.now())
useIntervalFn(() => {
  now.value = Date.now()
}, 60_000)
const due = computed(() => practice.items.filter((item) => item.dueAt <= now.value))
const done = ref(false)
const board = ref<{ hint: () => void; showSolution: () => Promise<void> } | null>(null)
function next(all = false): void {
  const item = [...(all ? practice.items : due.value)]
    .sort((a, b) => a.dueAt - b.dueAt)
    .find((item) => done.value || item.id !== current.value?.id)
  current.value = item ? practice.puzzle(item) : null
  done.value = false
}
</script>
<template>
  <div>
    <div class="card mb-4">
      <h2 class="section-title">Practice your mistakes</h2>
      <p class="section-hint">{{ due.length }} due · {{ practice.items.length }} saved</p>
      <p v-if="practice.error" role="alert">{{ practice.error }}</p>
      <UEmpty
        v-if="!practice.items.length"
        variant="naked"
        icon="i-lucide-book-open"
        title="Build your mistake practice"
        description="Save positions from a game review to practice them here."
        :actions="[
          {
            label: 'Analyze a game',
            icon: 'i-lucide-microscope',
            onClick: () => app.selectPage('analysis'),
          },
        ]"
      />
      <p v-else-if="!due.length && !current" class="section-hint mt-3" role="status">
        You're caught up.
      </p>
      <div v-if="practice.items.length" class="mt-3 flex gap-2">
        <UButton :disabled="!due.length" @click="next()">{{
          current ? 'Next due position' : 'Start practice'
        }}</UButton>
        <UButton
          variant="outline"
          color="neutral"
          :disabled="!practice.items.length"
          @click="next(true)"
          >Review early</UButton
        >
      </div>
    </div>
    <PuzzleBoard
      v-if="current"
      ref="board"
      :key="current.id"
      :puzzle="current"
      @outcome="practice.answered(current!.id, $event)"
      @done="done = true"
    />
    <div v-if="current && !done" class="mt-3 flex gap-2">
      <UButton variant="outline" color="neutral" @click="board?.hint()"
        >Hint: piece to move</UButton
      >
      <UButton variant="ghost" color="neutral" @click="board?.showSolution()"
        >Show engine line</UButton
      >
    </div>
    <p v-if="done" role="status" class="mt-3">
      Position completed. Your next revisit has been scheduled.
    </p>
  </div>
</template>
