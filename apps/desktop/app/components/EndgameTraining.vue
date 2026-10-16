<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import type { RunSummary } from '@kchess/contracts/types'
import { ENDGAME_DRILLS } from '@kchess/rules/endgames'
import { COURSE_GROUPS, LESSON_COURSES, courseProgress } from '@kchess/rules/lessons'
import { ENDGAME_VIEW_KEY } from '../utils/storageKeys'
import { courseIcon } from '../utils/courseIcons'
import EndgameDrills from './EndgameDrills.vue'
import EndgameLesson from './EndgameLesson.vue'

/** Practice → Endgames: the checkmate and endgame courses, and the drills against Stockfish. */
const view = useLocalStorage(ENDGAME_VIEW_KEY, 'courses')
const lessons = ref<RunSummary | null>(null)
const drills = ref<RunSummary | null>(null)
const course = computed(() => LESSON_COURSES.find((entry) => entry.id === view.value))

const best = (summary: RunSummary | null, id: string): boolean =>
  (summary?.best[id]?.score ?? 0) > 0
const cards = computed(() =>
  COURSE_GROUPS.map((group) => ({
    group,
    courses: LESSON_COURSES.filter((entry) => entry.group === group).map((entry) => ({
      ...entry,
      progress: courseProgress(entry, (id) => best(lessons.value, id)),
    })),
  })),
)
const drillProgress = computed(() => ({
  done: ENDGAME_DRILLS.filter((drill) => best(drills.value, drill.id)).length,
  total: ENDGAME_DRILLS.length,
}))

async function load(): Promise<void> {
  try {
    ;[lessons.value, drills.value] = await Promise.all([
      window.kchess.runSummary('lesson'),
      window.kchess.runSummary('endgame'),
    ])
  } catch (cause) {
    console.warn('[endgame-training] Could not load progress:', cause)
  }
}
onMounted(load)

function back(): void {
  view.value = 'courses'
  void load()
}
</script>

<template>
  <EndgameLesson
    v-if="course"
    :key="course.id"
    :course="course"
    :summary="lessons"
    @back="back"
    @saved="lessons = $event"
  />
  <EndgameDrills v-else-if="view === 'drills'" @back="back" />
  <div v-else class="course-groups">
    <section v-for="entry in cards" :key="entry.group" class="course-group">
      <h2 class="section-title">{{ entry.group }}</h2>
      <div class="course-grid">
        <button
          v-for="item in entry.courses"
          :key="item.id"
          type="button"
          class="course-card"
          @click="view = item.id"
        >
          <UIcon :name="courseIcon(item.id)" class="course-icon" />
          <span class="course-text">
            <span class="course-title">{{ item.title }}</span>
            <span class="course-subtitle muted">{{ item.subtitle }}</span>
          </span>
          <span
            class="course-badge"
            :class="{ done: item.progress.done === item.progress.total }"
            :aria-label="`${item.progress.done} of ${item.progress.total} chapters done`"
            >{{
              item.progress.done === item.progress.total
                ? 'Done'
                : `${item.progress.done} / ${item.progress.total}`
            }}</span
          >
        </button>
      </div>
    </section>
    <section class="course-group">
      <h2 class="section-title">Drills</h2>
      <div class="course-grid">
        <button type="button" class="course-card" @click="view = 'drills'">
          <UIcon name="i-lucide-swords" class="course-icon" />
          <span class="course-text">
            <span class="course-title">Endgame drills</span>
            <span class="course-subtitle muted">Win or hold against Stockfish</span>
          </span>
          <span
            class="course-badge"
            :class="{ done: drillProgress.done === drillProgress.total }"
            :aria-label="`${drillProgress.done} of ${drillProgress.total} drills done`"
            >{{
              drillProgress.done === drillProgress.total
                ? 'Done'
                : `${drillProgress.done} / ${drillProgress.total}`
            }}</span
          >
        </button>
      </div>
    </section>
  </div>
</template>

<style scoped>
.course-groups {
  display: flex;
  flex-direction: column;
  gap: 24px;
}
.course-group {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.course-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 12px;
}
.course-card {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 14px 16px;
  border: 1px solid var(--ui-border);
  border-radius: 12px;
  background: var(--ui-bg-elevated);
  color: inherit;
  text-align: left;
}
.course-card:hover {
  border-color: var(--ui-primary);
}
.course-icon {
  flex-shrink: 0;
  width: 40px;
  height: 40px;
  padding: 8px;
  border-radius: 10px;
  background: color-mix(in srgb, var(--ui-primary) 16%, transparent);
  color: var(--ui-primary);
}
.course-text {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
}
.course-title {
  font-size: 15px;
  font-weight: 600;
}
.course-subtitle {
  font-size: 13px;
}
.course-badge {
  flex-shrink: 0;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--ui-bg-accented);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.course-badge.done {
  background: color-mix(in srgb, var(--ui-success) 18%, transparent);
  color: var(--ui-success);
  font-weight: 600;
}
</style>
