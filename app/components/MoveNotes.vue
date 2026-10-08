<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import { useAnalysisStore } from '../stores/analysis'
import { MOVE_GLYPHS, moveGlyph, moveNumber, setMoveGlyph } from '../../src/shared/analysisTree'

/**
 * Notes on the move shown: a comment and an annotation glyph, written right under the move list
 * (and shown in it), so annotating never means leaving the board.
 */
const analysis = useAnalysisStore()
const { node, path } = storeToRefs(analysis)

const field = ref<HTMLTextAreaElement | null>(null)
const editing = ref(false)
const comment = computed({
  get: () => (node.value.comments ?? []).join('\n\n'),
  set: (text: string) => {
    node.value.comments = text.trim() ? [text] : undefined
  },
})
const glyph = computed(() => moveGlyph(node.value))
const subject = computed(() =>
  path.value
    ? `${moveNumber(node.value.ply - 1, true)} ${node.value.san}`
    : 'the starting position',
)
const open = computed(() => editing.value || Boolean(comment.value))

async function focus(): Promise<void> {
  editing.value = true
  await nextTick()
  field.value?.focus()
}
function toggleGlyph(nag: number): void {
  setMoveGlyph(node.value, glyph.value?.nag === nag ? undefined : nag)
}
function keydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') (event.target as HTMLElement).blur()
}
defineExpose({ focus })
</script>

<template>
  <section class="move-notes" aria-label="Notes on this move">
    <div class="notes-head">
      <span class="notes-subject">{{ subject }}</span>
      <div v-if="path" class="glyphs" role="group" aria-label="Annotate this move">
        <UTooltip v-for="entry in MOVE_GLYPHS" :key="entry.nag" :text="entry.label">
          <button
            type="button"
            class="glyph"
            :class="{ on: glyph?.nag === entry.nag }"
            :aria-label="entry.label"
            :aria-pressed="glyph?.nag === entry.nag"
            @click="toggleGlyph(entry.nag)"
          >
            {{ entry.glyph }}
          </button>
        </UTooltip>
      </div>
    </div>
    <textarea
      v-if="open"
      ref="field"
      v-model="comment"
      class="notes-field"
      rows="2"
      maxlength="10000"
      :aria-label="`Comment on ${subject}`"
      placeholder="What's the idea here?"
      @focus="editing = true"
      @blur="editing = false"
      @keydown="keydown"
    />
    <button v-else type="button" class="notes-add" @click="focus">
      <UIcon name="i-lucide-message-square-plus" />
      Add a comment
      <UKbd value="C" size="sm" class="ml-auto" />
    </button>
  </section>
</template>

<style scoped>
.move-notes {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.notes-head {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 26px;
}
.notes-subject {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  color: var(--ui-text-muted);
  font-size: 12px;
  font-weight: 600;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.glyphs {
  display: flex;
  gap: 2px;
}
.glyph {
  min-width: 26px;
  height: 24px;
  padding: 0 4px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--ui-text-muted);
  font: inherit;
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
}
.glyph:hover {
  background: var(--ui-bg-accented);
  color: var(--ui-text);
}
.glyph.on {
  border-color: color-mix(in srgb, var(--ui-primary) 60%, transparent);
  background: color-mix(in srgb, var(--ui-primary) 22%, transparent);
  color: var(--ui-text-highlighted);
}
.notes-field {
  width: 100%;
  min-height: 56px;
  max-height: 160px;
  padding: 8px 10px;
  border: 1px solid var(--ui-border);
  border-radius: 8px;
  background: var(--ui-bg);
  color: inherit;
  font: inherit;
  font-size: 13px;
  resize: vertical;
}
.notes-field:focus {
  outline: 2px solid color-mix(in srgb, var(--ui-primary) 60%, transparent);
  outline-offset: -1px;
}
.notes-add {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 7px 10px;
  border: 1px dashed var(--ui-border-accented);
  border-radius: 8px;
  background: transparent;
  color: var(--ui-text-muted);
  font: inherit;
  font-size: 13px;
  cursor: text;
}
.notes-add:hover {
  border-color: var(--ui-primary);
  color: var(--ui-text);
}
</style>
