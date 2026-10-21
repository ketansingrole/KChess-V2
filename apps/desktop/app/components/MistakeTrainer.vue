<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import { LEARNED_STREAK, useMistakeStore } from '../stores/mistakes'
import type { Puzzle } from '@kchess/contracts/types'
import type { MistakeGame } from '@kchess/rules/library'
import { formatGameDate } from '../utils/games'
import { judgmentCounts } from '../utils/review'
import { usePracticeGame } from '../composables/usePracticeGame'
const app = useKChessStore()
const practice = useMistakeStore()
onMounted(() => {
  practice
    .loadGames()
    .catch((cause: unknown) => console.warn('[mistake-trainer] Loading the games failed:', cause))
})
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
const learned = computed(
  () => practice.items.filter((item) => item.streak >= LEARNED_STREAK).length,
)

/* ── The games the positions come from ─────────────────────────────── */

const practiseGame = usePracticeGame()
const GAMES_SHOWN = 8
const showAll = ref(false)
function title(game: MistakeGame): string {
  const { white, black, player } = game
  if (player) {
    const opponent = player === 'white' ? black : white
    return opponent ? `vs ${opponent}` : `You as ${player}`
  }
  return white && black ? `${white} – ${black}` : 'Analysis board game'
}
const games = computed(() =>
  practice.games.map((game) => {
    const items = practice.items.filter((item) => item.id.startsWith(`${game.key}:`))
    const count = (judgment: string) => items.filter((item) => item.judgment === judgment).length
    return {
      game,
      title: title(game),
      date: formatGameDate(game.playedAt),
      positions: items.length,
      due: items.filter((item) => item.dueAt <= now.value).length,
      counts: judgmentCounts({
        blunder: count('blunder'),
        mistake: count('mistake'),
        inaccuracy: count('inaccuracy'),
      }),
    }
  }),
)
const shownGames = computed(() => (showAll.value ? games.value : games.value.slice(0, GAMES_SHOWN)))

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
  <div v-else class="mistakes-home">
    <div class="card">
      <h2 class="section-title">Practice your mistakes</h2>
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
        <dl class="mistake-stats">
          <div>
            <dt>Due now</dt>
            <dd>{{ due.length }}</dd>
          </div>
          <div>
            <dt>Learning</dt>
            <dd>{{ practice.items.length - learned }}</dd>
          </div>
          <div>
            <dt>Learned</dt>
            <dd>{{ learned }}</dd>
          </div>
        </dl>
        <p v-if="!due.length" class="section-hint mt-3" role="status">You're caught up.</p>
        <div class="mt-3 flex gap-2">
          <UButton :disabled="!due.length" @click="next()">Practice due positions</UButton>
          <UButton variant="outline" color="neutral" @click="next(true)">Review early</UButton>
        </div>
      </template>
    </div>

    <section v-if="games.length" class="card" aria-labelledby="mistake-games-title">
      <div class="games-head">
        <h2 id="mistake-games-title" class="section-title">Games</h2>
        <UButton
          v-if="app.data.accounts.length"
          size="xs"
          variant="link"
          color="neutral"
          icon="i-lucide-chart-pie"
          @click="app.selectPage('insights')"
          >Lichess insights</UButton
        >
      </div>
      <ul class="mistake-games">
        <li v-for="row in shownGames" :key="row.game.key">
          <!-- Your side; the slot stays when it is not known, so the titles line up. -->
          <span :class="['side-dot', row.game.player ?? 'unknown']" aria-hidden="true" />
          <span class="game-text">
            <strong class="game-title">{{ row.title }}</strong>
            <span class="game-sub">
              {{ row.date }} · {{ row.positions }}
              {{ row.positions === 1 ? 'position' : 'positions'
              }}<template v-if="row.due"> · {{ row.due }} due</template>
            </span>
          </span>
          <span class="game-counts">
            <span v-for="count in row.counts" :key="count.judgment" :class="count.judgment">{{
              count.text
            }}</span>
          </span>
          <UButton
            size="sm"
            variant="outline"
            color="neutral"
            :aria-label="`Practice ${row.title}, ${row.date}`"
            @click="practiseGame(row.game)"
            >Practice</UButton
          >
        </li>
      </ul>
      <UButton
        v-if="games.length > GAMES_SHOWN && !showAll"
        size="xs"
        variant="link"
        color="neutral"
        @click="showAll = true"
        >Show all {{ games.length }} games</UButton
      >
    </section>
  </div>
</template>

<style scoped>
.mistakes-home {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.mistake-stats {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 12px;
  max-width: 420px;
  margin: 12px 0 0;
}
.mistake-stats dt {
  color: var(--ui-text-muted);
  font-size: 12px;
  font-weight: 500;
}
.mistake-stats dd {
  margin: 2px 0 0;
  font-size: 26px;
  font-weight: 750;
  letter-spacing: -0.03em;
  font-variant-numeric: tabular-nums;
}
.games-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.mistake-games {
  margin: 8px 0 0;
  padding: 0;
  list-style: none;
}
.mistake-games li {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 0;
}
.mistake-games li + li {
  border-top: 1px solid var(--ui-border);
}
.side-dot.unknown {
  visibility: hidden;
}
.game-text {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}
.game-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13.5px;
}
.game-sub {
  color: var(--ui-text-muted);
  font-size: 12px;
}
.game-counts {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 2px 10px;
  font-size: 12px;
  font-weight: 600;
}
.game-counts .inaccuracy {
  color: var(--judgment-inaccuracy);
}
.game-counts .mistake {
  color: var(--judgment-mistake);
}
.game-counts .blunder {
  color: var(--judgment-blunder);
}
@container page (max-width: 520px) {
  .game-counts {
    display: none;
  }
}
</style>
