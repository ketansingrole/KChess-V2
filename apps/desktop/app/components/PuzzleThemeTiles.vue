<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { PUZZLE_THEME_GROUPS } from '../utils/puzzleThemes'

/**
 * Checkmate patterns, endgames and tactics as practice: each tile opens the puzzle trainer on that theme.
 * Practice online fetches puzzles from Lichess but reports nothing; Offline needs the local database.
 */
const puzzles = usePuzzleStore()
const { db } = storeToRefs(puzzles)
onMounted(() => void puzzles.refreshDb())

const groups = computed(() =>
  PUZZLE_THEME_GROUPS.filter((group) =>
    ['Mate patterns', 'Endgames', 'Tactical motifs', 'Advanced tactics'].includes(group.label),
  ),
)
</script>

<template>
  <div class="card">
    <h2 class="sr-only">Practice by theme</h2>
    <div v-for="group in groups" :key="group.label" class="theme-section">
      <h3 class="section-title">{{ group.label }}</h3>
      <div class="theme-tiles">
        <div v-for="theme in group.themes" :key="theme.key" class="theme-tile">
          <strong :title="theme.hint">{{ theme.name }}</strong>
          <div class="theme-tile-actions">
            <UButton size="xs" variant="soft" @click="puzzles.startTheme(theme.key, 'practice')"
              >Practice</UButton
            >
            <UTooltip
              :text="
                db?.installed
                  ? 'Practice offline with the puzzles stored on this computer'
                  : 'Offline: needs the puzzle database (Puzzles → Storm · Streak · Rush)'
              "
            >
              <UButton
                size="xs"
                variant="outline"
                color="neutral"
                icon="i-lucide-hard-drive"
                aria-label="Practice offline"
                :disabled="!db?.installed"
                @click="puzzles.startTheme(theme.key, 'offline')"
              />
            </UTooltip>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.theme-section + .theme-section {
  margin-top: 18px;
}
.theme-tiles {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
  gap: 8px;
  margin-top: 8px;
}
.theme-tile {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 10px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg);
}
.theme-tile strong {
  display: block;
  font-size: 13px;
}
.theme-tile-actions {
  display: flex;
  gap: 6px;
}
</style>
