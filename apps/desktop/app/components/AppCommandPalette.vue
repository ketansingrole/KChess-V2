<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import type { CommandPaletteGroup } from '@nuxt/ui'
import { labelSearchResults, observeUiAccessibility } from '../utils/uiAccessibility'
import type { QuickPickItem } from '../composables/useQuickPick'
import type { QuickPickMode } from '../utils/quickPick'

const props = withDefaults(
  defineProps<{ groups?: CommandPaletteGroup<QuickPickItem>[]; mode?: QuickPickMode }>(),
  { groups: () => [], mode: 'all' },
)
/** The top bar's search box is the palette's input; while open, results expand beneath it. */
const open = defineModel<boolean>('open', { default: false })
const query = defineModel<string>('query', { default: '' })

const placeholders = {
  all: 'Search pages and commands (type ? for help)',
  commands: 'Type a command…',
  players: 'Search players you follow, or type a Lichess username',
  settings: 'Search settings…',
  help: 'Pick how to search',
}

const root = ref<HTMLElement | null>(null)
const input = (): HTMLInputElement | null | undefined => root.value?.querySelector('input')

/** Focus the box with the caret at the end, after a mode prefix or a shortcut. */
async function placeCaret(): Promise<void> {
  await nextTick()
  const box = input()
  if (!box) return
  box.focus()
  box.setSelectionRange(query.value.length, query.value.length)
}
watch(open, (isOpen) => {
  if (isOpen) void placeCaret()
  else if (root.value?.contains(document.activeElement))
    (document.activeElement as HTMLElement).blur()
})
watch(query, (value, previous) => {
  if (value.length === 1 && !previous.startsWith(value)) void placeCaret()
})

/** Anything outside the box closes it, like VS Code's quick pick. */
function onPointerDown(event: PointerEvent): void {
  if (open.value && !root.value?.contains(event.target as Node)) open.value = false
}
function onFocusOut(event: FocusEvent): void {
  const next = event.relatedTarget as Node | null
  if (next && !root.value?.contains(next)) open.value = false
}
function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape' && open.value) {
    event.preventDefault()
    event.stopPropagation()
    open.value = false
  } else if (!open.value && event.key !== 'Tab') {
    open.value = true
  } else if (event.key === 'Backspace' && query.value.length === 1 && props.mode !== 'all') {
    // Backspace on an empty mode drops back to plain search.
    event.preventDefault()
    query.value = ''
  }
}

let stop: (() => void) | undefined
onMounted(() => {
  if (root.value) stop = observeUiAccessibility(root.value, labelSearchResults)
  document.addEventListener('pointerdown', onPointerDown, true)
})
onUnmounted(() => {
  stop?.()
  document.removeEventListener('pointerdown', onPointerDown, true)
})
</script>

<template>
  <div
    ref="root"
    class="quickpick"
    :class="{ open }"
    data-ui-accessibility="search"
    @keydown.capture="onKeydown"
    @focusin="open || (open = true)"
    @focusout="onFocusOut"
  >
    <div class="quickpick-panel">
      <LazyUCommandPalette
        v-model:search-term="query"
        :groups="open ? groups : []"
        :fuse="{ resultLimit: 100 }"
        :placeholder="open ? placeholders[props.mode] : 'Search…'"
        :input="{
          'aria-label': 'Search pages and commands',
          'aria-keyshortcuts': 'Meta+K Control+K',
        }"
        icon="i-lucide-search"
        :autofocus="false"
        :back="false"
        size="sm"
        class="kchess-search-palette"
        :ui="{
          root: 'divide-y-0 min-h-0',
          input: 'kchess-quickpick-input',
          content: open ? 'quickpick-results' : 'hidden',
          viewport: 'divide-y-0 kchess-quickpick-list',
          group: 'p-0 py-1 kchess-quickpick-group',
          label: 'px-2.5 pt-1.5 pb-1 text-[11px] font-medium text-muted',
          item: 'px-2.5 py-1.5 gap-2.5 text-[13px] rounded-md before:inset-x-0 before:inset-y-0 data-highlighted:before:bg-primary/12',
          itemLeadingIcon: 'size-4 text-muted group-data-highlighted:text-primary',
          itemLabel: 'space-x-2',
          itemLabelBase:
            'text-default [&>mark]:bg-transparent [&>mark]:text-primary [&>mark]:font-semibold',
          itemLabelSuffix: 'text-[12px] text-dimmed',
          empty: 'py-6 text-[13px]',
          footer: 'p-0',
        }"
      >
        <template #item-trailing="{ item }">
          <span v-if="item.hint" class="quickpick-hint">{{ item.hint }}</span>
          <span v-if="item.kbds?.length" class="quickpick-kbds">
            <UKbd
              v-for="(kbd, index) in item.kbds as string[]"
              :key="index"
              :value="kbd"
              size="sm"
            />
          </span>
        </template>
        <template #empty="{ searchTerm }">
          No results for “{{ searchTerm.replace(/^[>@#?]/, '').trim() }}”
        </template>
        <template v-if="open" #footer>
          <div class="quickpick-footer" aria-hidden="true">
            <span><UKbd value="↑" size="sm" /><UKbd value="↓" size="sm" /> navigate</span>
            <span><UKbd value="↵" size="sm" /> open</span>
            <span><UKbd value="esc" size="sm" /> close</span>
            <span class="ms-auto"
              ><UKbd value=">" size="sm" /> commands <UKbd value="@" size="sm" /> players
              <UKbd value="#" size="sm" /> settings</span
            >
          </div>
        </template>
      </LazyUCommandPalette>
      <span v-if="!open" class="quickpick-keys" aria-hidden="true">
        <UKbd value="meta" size="sm" /><UKbd value="K" size="sm" />
      </span>
    </div>
  </div>
</template>

<style scoped>
.quickpick-hint {
  font-size: 12px;
  color: var(--ui-text-dimmed);
  white-space: nowrap;
}
.quickpick-kbds {
  display: inline-flex;
  gap: 2px;
}
.quickpick-footer {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 14px;
  padding: 6px 10px;
  border-top: 1px solid var(--ui-border);
  font-size: 11px;
  color: var(--ui-text-muted);
}
.quickpick-footer span {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}
</style>
