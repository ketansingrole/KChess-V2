<script setup lang="ts">
import { computed, ref } from 'vue'
import { refDebounced } from '@vueuse/core'
import { openingTree, type OpeningNode } from '@kchess/rules/openings'
import OpeningsTreeNode from './OpeningsTreeNode.vue'
import { openingLink } from './openingLink'

/** Every named opening by family, and a search over their names and ECO codes. */
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
</script>

<template>
  <div class="card opening-browser">
    <UInput
      v-model="query"
      icon="i-lucide-search"
      placeholder="Search openings or ECO codes"
      aria-label="Search openings"
      class="w-full"
    />
    <template v-if="matches">
      <p v-if="!matches.length" class="muted text-sm" role="status">No openings found.</p>
      <ul v-else class="mt-3" aria-label="Search results">
        <li v-for="entry in matches.slice(0, LIMIT)" :key="entry.name" class="opening-result">
          <NuxtLink :to="openingLink(entry.moves)" class="opening-result-name">{{
            entry.name
          }}</NuxtLink>
          <span class="muted tabular text-xs">{{ entry.eco }}</span>
        </li>
      </ul>
      <p v-if="matches.length > LIMIT" class="muted text-xs mt-2" role="status">
        {{ LIMIT }} of {{ matches.length.toLocaleString() }} openings
      </p>
    </template>
    <ul v-else class="mt-3" aria-label="Opening families">
      <OpeningsTreeNode v-for="node in tree" :key="node.name" :node="node" />
    </ul>
  </div>
</template>

<style scoped>
.opening-browser {
  max-width: 760px;
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
</style>
