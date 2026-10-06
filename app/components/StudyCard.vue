<script setup lang="ts">
import { computed } from 'vue'
import { summarizeStudy } from '../utils/studies'
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
    <div class="p-3 flex flex-col gap-2">
      <div class="flex items-start gap-2">
        <button
          type="button"
          class="font-semibold text-left flex-1 min-w-0 break-words"
          @click="emit('open')"
        >
          {{ name }}
        </button>
        <slot name="menu" />
      </div>
      <p
        v-if="summary?.players || (summary?.event && summary.event !== name)"
        class="text-xs muted truncate"
      >
        {{
          [summary?.players, summary?.event !== name ? summary?.event : '']
            .filter(Boolean)
            .join(' · ')
        }}
      </p>
      <p class="text-xs muted">
        {{ location
        }}<template v-if="chapterCount">
          · {{ chapterCount }} {{ chapterCount === 1 ? 'chapter' : 'chapters' }}</template
        >
      </p>
      <p class="text-xs muted">Edited {{ timeAgo(updatedAt) }}</p>
      <slot />
    </div>
  </article>
</template>
<style scoped>
.study-card {
  overflow: hidden;
  border: 1px solid var(--ui-border);
  border-radius: 12px;
  background: var(--ui-bg-elevated);
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
