<script setup lang="ts">
import type { ActivityItem } from '@kchess/contracts/types'

/** Lines of Lichess activity under day headings: a player's activity, or an account's timeline. */
defineProps<{ groups: { label: string; items: ActivityItem[] }[] }>()

/** The text before and after the player it names, so the name links to their profile. */
function named(item: ActivityItem): [string, string] | null {
  const at = item.username ? item.text.indexOf(item.username) : -1
  if (at < 0 || !item.username) return null
  return [item.text.slice(0, at), item.text.slice(at + item.username.length)]
}

const ICONS: Record<string, string> = {
  games: 'i-lucide-swords',
  puzzles: 'i-lucide-puzzle',
  tournament: 'i-lucide-trophy',
  follow: 'i-lucide-user-plus',
  study: 'i-lucide-book-open',
  team: 'i-lucide-users',
  post: 'i-lucide-message-square',
  simul: 'i-lucide-layout-grid',
  patron: 'i-lucide-heart',
  stream: 'i-lucide-radio',
  correspondence: 'i-lucide-mail',
}
</script>

<template>
  <div class="activity-list text-sm">
    <section v-for="group in groups" :key="group.label">
      <h3 class="muted text-xs mb-1">{{ group.label }}</h3>
      <ul>
        <li v-for="(item, index) in group.items" :key="index">
          <UIcon :name="ICONS[item.kind] ?? 'i-lucide-dot'" class="muted shrink-0" />
          <span v-if="named(item)"
            >{{ named(item)![0] }}<PlayerLink :username="item.username!" />{{
              named(item)![1]
            }}</span
          >
          <span v-else>{{ item.text }}</span>
        </li>
      </ul>
    </section>
  </div>
</template>

<style scoped>
.activity-list {
  display: grid;
  gap: 14px;
}
.activity-list ul {
  display: grid;
  gap: 6px;
}
.activity-list li {
  display: flex;
  align-items: baseline;
  gap: 8px;
}
</style>
