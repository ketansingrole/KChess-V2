<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useLocalStorage } from '@vueuse/core'

/** The game chat, docked at the bottom right like a messenger window so it never pushes the game around. */
const store = useKChessStore()
const { chat, onlinePhase, onlineId } = storeToRefs(store)

const open = useLocalStorage('kchess:chat-open', false)
const showSpectators = useLocalStorage('kchess:chat-spectators', false)
const draft = ref('')
const log = ref<HTMLElement | null>(null)
/** Lichess's own quick messages. */
const PRESETS = [
  'Hello',
  'Good luck',
  'Have fun!',
  'You too!',
  'Good game',
  'Well played',
  'Thank you',
]

const lines = computed(() =>
  chat.value.filter((line) => showSpectators.value || line.room === 'player'),
)
/** How many shown lines had been read when the window was last open; it resets with each game. */
const read = ref(0)
watch(onlineId, () => (read.value = 0))
watch(
  [() => lines.value.length, open],
  async ([count, isOpen]) => {
    if (count < read.value) read.value = 0
    if (!isOpen) return
    read.value = count
    await nextTick()
    if (log.value) log.value.scrollTop = log.value.scrollHeight
  },
  { immediate: true },
)
const me = computed(() => (store.onlineAccount || store.activeOnlineAccount).toLowerCase())
const unread = computed(
  () => lines.value.slice(read.value).filter((line) => line.user.toLowerCase() !== me.value).length,
)

async function send(text = draft.value): Promise<void> {
  const message = text.trim()
  if (!message) return
  if (await store.sendChat(message)) {
    if (text === draft.value) draft.value = ''
  }
}
</script>

<template>
  <Teleport to="body">
    <section class="chat-dock" :class="{ open }" aria-label="Game chat">
      <header class="chat-dock-head">
        <button
          type="button"
          class="chat-dock-toggle"
          :aria-expanded="open"
          aria-controls="chat-dock-body"
          @click="open = !open"
        >
          <UIcon name="i-lucide-message-circle" />
          <span class="font-semibold">Chat</span>
          <UBadge v-if="unread" size="sm" color="primary" class="tabular">{{ unread }}</UBadge>
          <span class="flex-1" />
          <UIcon :name="open ? 'i-lucide-chevron-down' : 'i-lucide-chevron-up'" />
        </button>
        <UTooltip v-if="open" text="Also show messages from people watching">
          <UButton
            size="xs"
            :variant="showSpectators ? 'soft' : 'ghost'"
            :color="showSpectators ? 'primary' : 'neutral'"
            icon="i-lucide-users"
            aria-label="Spectator messages"
            :aria-pressed="showSpectators"
            @click="showSpectators = !showSpectators"
          />
        </UTooltip>
      </header>
      <div v-show="open" id="chat-dock-body" class="chat-dock-body">
        <div ref="log" class="chat-log" role="log" aria-live="polite">
          <p v-if="!lines.length" class="muted">No messages.</p>
          <p v-for="(line, index) in lines" :key="index" class="chat-line" :class="line.room">
            <span class="chat-user">{{ line.user }}</span
            >{{ line.text }}
          </p>
        </div>
        <div v-if="onlinePhase === 'playing' || onlinePhase === 'finished'" class="chat-presets">
          <UButton
            v-for="preset in PRESETS"
            :key="preset"
            size="xs"
            variant="soft"
            color="neutral"
            @click="send(preset)"
            >{{ preset }}</UButton
          >
        </div>
        <form class="mt-2 flex gap-2" @submit.prevent="send()">
          <UInput
            v-model="draft"
            size="sm"
            :maxlength="140"
            placeholder="Message your opponent"
            aria-label="Chat message"
            autocomplete="off"
            class="flex-1"
          />
          <UButton
            type="submit"
            size="sm"
            icon="i-lucide-send"
            aria-label="Send"
            :disabled="!draft.trim()"
          />
        </form>
      </div>
    </section>
  </Teleport>
</template>

<style scoped>
.chat-dock {
  position: fixed;
  right: 1rem;
  bottom: 0;
  z-index: 40;
  width: 18rem;
  max-width: calc(100vw - 2rem);
  border: 1px solid var(--ui-border);
  border-bottom: 0;
  border-radius: var(--ui-radius) var(--ui-radius) 0 0;
  background: var(--ui-bg-elevated);
  box-shadow: 0 -4px 24px rgb(0 0 0 / 0.25);
}
.chat-dock.open {
  width: 20rem;
}
.chat-dock-head {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  padding-right: 0.5rem;
}
.chat-dock-toggle {
  display: flex;
  flex: 1;
  align-items: center;
  gap: 0.5rem;
  /* Slim enough to sit in the page's bottom padding, clear of the side panel. */
  height: 2.25rem;
  padding: 0 0.75rem;
  font-size: 0.875rem;
  cursor: pointer;
}
.chat-dock-body {
  padding: 0 0.75rem 0.75rem;
  border-top: 1px solid var(--ui-border);
}
.chat-dock-body .chat-log {
  height: 14rem;
  max-height: 40vh;
}
</style>
