<script setup lang="ts">
import { computed } from 'vue'
import { summarizeStudy } from '../../../../core/src/domain/studies'
import { timeAgo } from '../utils/format'
const app = useKChessStore()
const props = defineProps<{
  name: string
  updatedAt: number
  pgn?: string
  chapterCount?: number
  location: string
}>()
const summary = computed(() => (props.pgn ? summarizeStudy(props.pgn) : null))
const emit = defineEmits<{ open: [] }>()
</script>
<template>
  <article class="study-card">
    <button type="button" class="study-preview" :aria-label="`Open ${name}`" @click="emit('open')">
      <ChessBoard
        v-if="summary"
        :fen="summary.fen"
        :theme="app.settings?.boardTheme"
        :piece-set="app.settings?.pieceSet"
        coordinates="none"
        animation="none"
        :interactive="false"
      />
      <UIcon v-else name="i-lucide-book-open" class="size-12 text-muted" />
    </button>
    <div class="study-body">
      <div class="flex items-start gap-2">
        <button type="button" class="study-title" :title="name" @click="emit('open')">
          <span class="study-title-text">{{ name }}</span>
        </button>
        <slot name="menu" />
      </div>
      <div class="study-details">
        <p class="study-location text-xs muted">
          {{ location
          }}<template v-if="chapterCount">
            · {{ chapterCount }} {{ chapterCount === 1 ? 'chapter' : 'chapters' }}</template
          >
        </p>
        <p
          v-if="summary?.players || (summary?.event && summary.event !== name)"
          class="study-description text-xs muted truncate"
        >
          {{
            [summary?.players, summary?.event !== name ? summary?.event : '']
              .filter(Boolean)
              .join(' · ')
          }}
        </p>
        <p class="study-edited text-xs muted">Edited {{ timeAgo(updatedAt) }}</p>
      </div>
      <div class="study-actions"><slot /></div>
    </div>
  </article>
</template>
<style scoped>
.study-card {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
  border: 1px solid var(--ui-border);
  border-radius: 12px;
  background: var(--ui-bg-elevated);
}
.study-body {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  padding: 12px;
  gap: 8px;
}
.study-title {
  flex: 1;
  min-width: 0;
  min-height: 2.8em;
  line-height: 1.4;
  font-weight: 600;
  text-align: left;
  display: flex;
  align-items: flex-start;
}
.study-title-text {
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow-wrap: anywhere;
}
.study-details {
  display: grid;
  grid-template-rows: repeat(3, 1.25em);
  gap: 8px;
  font-size: 0.75rem;
}
.study-location {
  grid-row: 1;
}
.study-description {
  grid-row: 2;
}
.study-edited {
  grid-row: 3;
}
.study-actions {
  display: flex;
  flex-direction: column;
  margin-top: auto;
  padding-top: 4px;
}
.study-preview {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  aspect-ratio: 1;
  background: var(--ui-bg);
  overflow: hidden;
  cursor: pointer;
}
.study-preview :deep(.cg-host) {
  width: 100%;
  pointer-events: none;
}
.study-preview :deep(.cg-wrap) {
  width: 100%;
  aspect-ratio: 1;
  pointer-events: none;
}
</style>
