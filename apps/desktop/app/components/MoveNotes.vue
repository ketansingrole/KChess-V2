<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useAnalysisStore } from '../stores/analysis'
import {
  MOVE_GLYPHS,
  commentClock,
  commentText,
  moveGlyph,
  moveNumber,
  setMoveGlyph,
  withCommentText,
} from '@kchess/rules/analysisTree'
import { formatClock } from '@kchess/rules/clock'

/**
 * Notes on the move shown: a comment and an annotation glyph, written right under the move list
 * (and shown in it), so annotating never means leaving the board.
 */
const analysis = useAnalysisStore()
const { node, path } = storeToRefs(analysis)

const field = ref<HTMLTextAreaElement | null>(null)
const editing = ref(false)
/** The move's comments as written; a game's clock times (`[%clk …]`) ride along unseen. */
const raw = computed(() => (node.value.comments ?? []).join('\n\n'))
const shown = computed(() =>
  (node.value.comments ?? []).map(commentText).filter(Boolean).join('\n\n'),
)
/** What the field holds: the text as typed, which the stored comment may have tidied. */
const comment = ref(shown.value)
watch(path, () => (comment.value = shown.value))
watch(shown, (text) => {
  if (!editing.value) comment.value = text
})
function write(text: string): void {
  comment.value = text
  const next = withCommentText(raw.value, text)
  node.value.comments = next ? [next] : undefined
}
const clock = computed(() => {
  const seconds = commentClock(raw.value)
  return seconds === undefined ? '' : formatClock(seconds * 1000)
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
      <span class="notes-subject"
        >{{ subject
        }}<span v-if="clock" class="notes-clock" title="Clock after this move">
          · {{ clock }}</span
        ></span
      >
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
      :value="comment"
      class="notes-field"
      rows="2"
      maxlength="10000"
      :aria-label="`Comment on ${subject}`"
      placeholder="What's the idea here?"
      @focus="editing = true"
      @blur="editing = false"
      @keydown="keydown"
      @input="write(($event.target as HTMLTextAreaElement).value)"
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
.notes-clock {
  font-weight: 400;
  font-variant-numeric: tabular-nums;
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
