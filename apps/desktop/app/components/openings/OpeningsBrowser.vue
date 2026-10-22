<script setup lang="ts">
import { computed, ref } from 'vue'
import { refDebounced, useLocalStorage } from '@vueuse/core'
import { sanHistory } from '@kchess/rules/chess'
import { openingAt, openingTree, type OpeningNode } from '@kchess/rules/openings'
import { Position } from '@kchess/rules/position'
import { STANDARD_SETUP } from '@kchess/rules/variant'
import StudyCard from '../StudyCard.vue'
import OpeningsTreeNode from './OpeningsTreeNode.vue'
import { openingLink } from './openingLink'
import { useRecentOpenings, useTrainOpening } from '../../composables/useOpenings'
import { OPENINGS_GROUP_KEY } from '../../utils/storageKeys'

/** The openings landing page: recent and popular openings, every family by first move, a search. */
const router = useRouter()
const train = useTrainOpening()

const tree = openingTree()
const named = (() => {
  const out: (OpeningNode & { moves: string[]; key: string })[] = []
  const walk = (nodes: OpeningNode[]): void => {
    for (const node of nodes) {
      if (node.moves)
        out.push({ ...node, moves: node.moves, key: fold(`${node.name} ${node.eco}`) })
      walk(node.children)
    }
  }
  walk(tree)
  return out.sort((a, b) => a.moves.length - b.moves.length || a.name.localeCompare(b.name))
})()
/** Search ignores case, accents and the kind of apostrophe. */
function fold(text: string): string {
  return text.normalize('NFD').replace(/\p{M}/gu, '').replace(/[’`]/g, "'").toLowerCase()
}
const LIMIT = 60
const query = ref('')
const search = refDebounced(query, 120)
const matches = computed(() => {
  const words = fold(search.value).split(/\s+/).filter(Boolean)
  if (!words.length) return null
  return named.filter((entry) => words.every((word) => entry.key.includes(word)))
})

/** An opening as a card shows it: its name and code, its position and its moves. */
interface OpeningEntry {
  name: string
  eco: string
  moves: string[]
  fen: string
  san: string
}
function entry(moves: readonly string[], name?: string): OpeningEntry | null {
  const steps = Position.initial().line(moves)
  const last = steps.at(-1)
  const line = steps.map((step) => step.uci)
  const opening = openingAt(STANDARD_SETUP, line)
  if (!last || !opening) return null
  const san = sanHistory(line)
    .map((move, index) => (index % 2 ? move : `${index / 2 + 1}.${move}`))
    .join(' ')
  return { name: name ?? opening.name, eco: opening.eco, moves: line, fen: last.fen, san }
}

/** Widely played openings, by the line that names each. */
const POPULAR = [
  'e2e4 c7c5',
  'e2e4 e7e6',
  'e2e4 c7c6',
  'e2e4 e7e5 g1f3 b8c6 f1b5',
  'e2e4 e7e5 g1f3 b8c6 f1c4',
  'e2e4 d7d5',
  'd2d4 d7d5 c2c4',
  'd2d4 d7d5 c2c4 c7c6',
  'd2d4 g8f6 c2c4 g7g6 b1c3',
  'd2d4 g8f6 c2c4 e7e6 b1c3 f8b4',
  'd2d4 d7d5 g1f3 g8f6 c1f4',
  'c2c4',
]
const popular = POPULAR.map((line) => entry(line.split(' '))).filter(
  (item): item is OpeningEntry => !!item,
)

const { recent } = useRecentOpenings()
const recentEntries = computed(() =>
  (Array.isArray(recent.value) ? recent.value : [])
    .map((choice) =>
      typeof choice?.name === 'string' && Array.isArray(choice.moves)
        ? entry(choice.moves, choice.name)
        : null,
    )
    .filter((item): item is OpeningEntry => !!item),
)

/** Every family under the first move of its shortest named line. */
const GROUPS = [
  { label: '1.e4', value: 'e4', first: ['e2e4'] },
  { label: '1.d4', value: 'd4', first: ['d2d4'] },
  { label: '1.c4 / 1.Nf3', value: 'flank', first: ['c2c4', 'g1f3'] },
  { label: 'Other', value: 'other', first: [] },
]
function firstMove(node: OpeningNode): string | undefined {
  return node.moves?.[0] ?? node.children.map(firstMove).find(Boolean)
}
function groupOf(node: OpeningNode): string {
  const move = firstMove(node)
  return GROUPS.find((group) => move && group.first.includes(move))?.value ?? 'other'
}
const group = useLocalStorage(OPENINGS_GROUP_KEY, 'e4')
const families = computed(() => tree.filter((node) => groupOf(node) === group.value))

function open(item: OpeningEntry): void {
  void router.push(openingLink(item.moves))
}
</script>

<template>
  <div class="opening-browser">
    <UInput
      v-model="query"
      icon="i-lucide-search"
      placeholder="Search openings or ECO codes"
      aria-label="Search openings"
      class="w-full"
    />
    <div v-if="matches" class="card">
      <p v-if="!matches.length" class="muted text-sm" role="status">No openings found.</p>
      <ul v-else aria-label="Search results">
        <li v-for="match in matches.slice(0, LIMIT)" :key="match.name" class="opening-result">
          <NuxtLink :to="openingLink(match.moves)" class="opening-result-name">{{
            match.name
          }}</NuxtLink>
          <span class="muted tabular text-xs">{{ match.eco }}</span>
        </li>
      </ul>
      <p v-if="matches.length > LIMIT" class="muted text-xs mt-2" role="status">
        {{ LIMIT }} of {{ matches.length.toLocaleString() }} openings
      </p>
    </div>
    <template v-else>
      <section v-if="recentEntries.length" aria-labelledby="openings-recent">
        <h2 id="openings-recent" class="section-title mb-2">Recent</h2>
        <ul class="recent-list" aria-label="Recent openings">
          <li v-for="item in recentEntries" :key="item.name">
            <NuxtLink :to="openingLink(item.moves)" class="recent-item">
              <span class="recent-board" aria-hidden="true">
                <ChessBoard preview :fen="item.fen" :interactive="false" />
              </span>
              <span class="recent-name">{{ item.name }}</span>
              <span class="muted tabular text-xs">{{ item.eco }}</span>
            </NuxtLink>
          </li>
        </ul>
      </section>
      <section aria-labelledby="openings-popular">
        <h2 id="openings-popular" class="section-title mb-2">Popular</h2>
        <ul class="card-grid popular-grid" aria-label="Popular openings">
          <li v-for="item in popular" :key="item.name">
            <StudyCard
              :name="item.name"
              :fen="item.fen"
              :location="item.eco"
              :description="item.san"
              @open="open(item)"
            >
              <UButton
                size="sm"
                variant="outline"
                color="neutral"
                icon="i-lucide-graduation-cap"
                block
                :aria-label="`Train ${item.name}`"
                @click="train(item)"
                >Train</UButton
              >
            </StudyCard>
          </li>
        </ul>
      </section>
      <section aria-labelledby="openings-all">
        <h2 id="openings-all" class="section-title mb-2">All openings</h2>
        <UTabs
          v-model="group"
          :items="GROUPS"
          :content="false"
          variant="pill"
          aria-label="First move"
          class="mb-3"
        />
        <div class="card">
          <ul aria-label="Opening families">
            <OpeningsTreeNode v-for="node in families" :key="node.name" :node="node" />
          </ul>
        </div>
      </section>
    </template>
  </div>
</template>

<style scoped>
.opening-browser {
  display: flex;
  flex-direction: column;
  gap: 24px;
  max-width: 1100px;
  margin: 0 auto;
}
.opening-result {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  padding-block: 4px;
}
.opening-result-name {
  min-width: 0;
}
.opening-result-name:hover {
  text-decoration: underline;
}
.popular-grid {
  --study-detail-rows: 2;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
}
.recent-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 8px;
}
.recent-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 10px 6px 6px;
  border: 1px solid var(--ui-border);
  border-radius: 10px;
  background: var(--ui-bg-elevated);
}
.recent-item:hover,
.recent-item:focus-visible {
  background: var(--ui-bg-accented);
}
.recent-board {
  flex: none;
  width: 44px;
  pointer-events: none;
}
.recent-board :deep(.cg-host) {
  --board-radius: 4px;
}
.recent-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
