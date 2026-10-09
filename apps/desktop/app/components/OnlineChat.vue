<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'

const store = useKChessStore()
const { chat, onlinePhase } = storeToRefs(store)

const draft = ref('')
const showSpectators = ref(false)
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

watch(
  () => chat.value.length,
  async () => {
    await nextTick()
    if (log.value) log.value.scrollTop = log.value.scrollHeight
  },
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
  <section class="panel-divider pt-3" aria-label="Game chat">
    <div class="flex items-center justify-between gap-2">
      <h3 class="section-title text-sm">Chat</h3>
      <USwitch v-model="showSpectators" size="xs" label="Spectators" />
    </div>
    <div ref="log" class="chat-log" role="log" aria-live="polite">
      <p v-if="!chat.length" class="muted">No messages.</p>
      <p
        v-for="(line, index) in chat.filter((l) => showSpectators || l.room === 'player')"
        :key="index"
        class="chat-line"
        :class="line.room"
      >
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
  </section>
</template>
