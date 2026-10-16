<script setup lang="ts">
import { ref } from 'vue'
import type { OpeningNode } from '@kchess/rules/openings'
import { openingLink } from './openingLink'

/** One entry of the openings tree, with its variations below it once opened. */
defineProps<{ node: OpeningNode }>()
const open = ref(false)
</script>

<template>
  <li>
    <div class="opening-row">
      <UButton
        v-if="node.children.length"
        size="xs"
        variant="ghost"
        color="neutral"
        :icon="open ? 'i-lucide-chevron-down' : 'i-lucide-chevron-right'"
        :aria-label="`${open ? 'Hide' : 'Show'} ${node.name} variations`"
        :aria-expanded="open"
        @click="open = !open"
      />
      <span v-else class="opening-row-spacer" />
      <NuxtLink v-if="node.moves" :to="openingLink(node.moves)" class="opening-link">{{
        node.label
      }}</NuxtLink>
      <button v-else type="button" class="opening-link" @click="open = !open">
        {{ node.label }}
      </button>
      <span v-if="node.eco" class="muted tabular text-xs">{{ node.eco }}</span>
    </div>
    <ul v-if="open" class="opening-children">
      <OpeningsTreeNode v-for="child in node.children" :key="child.name" :node="child" />
    </ul>
  </li>
</template>

<style scoped>
.opening-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 28px;
}
.opening-row-spacer {
  width: 24px;
  flex: none;
}
.opening-link {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
}
.opening-link:hover {
  text-decoration: underline;
}
.opening-children {
  margin-left: 14px;
  padding-left: 10px;
  border-left: 1px solid var(--ui-border);
}
</style>
