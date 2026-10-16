<script setup lang="ts">
/** Grouped practice items (endgame drills, lesson chapters) with a tick for each finished one. */
export interface TrainingListItem {
  id: string
  title: string
  done: boolean
  /** A short label at the end of the row (a level). */
  aside?: string
}

defineProps<{
  groups: { label?: string; items: TrainingListItem[] }[]
  activeId: string
}>()
const emit = defineEmits<{ select: [id: string] }>()
</script>

<template>
  <div v-for="(group, index) in groups" :key="group.label ?? index" class="training-group">
    <h3 v-if="group.label" class="section-title">{{ group.label }}</h3>
    <button
      v-for="entry in group.items"
      :key="entry.id"
      type="button"
      class="training-item"
      :class="{ active: entry.id === activeId }"
      :aria-current="entry.id === activeId ? 'true' : undefined"
      @click="emit('select', entry.id)"
    >
      <UIcon
        :name="entry.done ? 'i-lucide-circle-check' : 'i-lucide-circle'"
        :class="entry.done ? 'text-success' : 'muted'"
        :aria-label="entry.done ? 'Done' : undefined"
      />
      <span class="training-title">{{ entry.title }}</span>
      <span v-if="entry.aside" class="training-aside muted text-xs">{{ entry.aside }}</span>
    </button>
  </div>
</template>

<style scoped>
.training-group {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.training-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  border: 1px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: inherit;
  font-size: 13px;
  text-align: left;
}
.training-item > :first-child {
  flex-shrink: 0;
}
.training-aside {
  margin-left: auto;
}
.training-item:hover {
  background: var(--ui-bg-accented);
}
.training-item.active {
  border-color: var(--ui-primary);
  background: color-mix(in srgb, var(--ui-primary) 10%, transparent);
}
</style>
