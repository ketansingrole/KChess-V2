<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import {
  VoiceCapture,
  type VoiceErrorKind,
  type VoiceResult,
  type VoiceState,
} from '../utils/voiceCapture'
import { LAUNCHER_PERMISSION_HINT, useMicrophoneAccess } from '../utils/microphone'

const props = defineProps<{
  active: boolean
  contextKey: string
  grammar: readonly string[]
  hint: string
  feedback?: string
  disabled?: boolean
  allowPushTalk?: boolean
  /** One row (toggle, status line, hold button) for tight spots such as under the board. */
  compact?: boolean
  /** Pass low-confidence phrases on (marked `unclear`) for the parent to double-check. */
  acceptUnclear?: boolean
}>()
const enabled = defineModel<boolean>('enabled', { default: false })
const emit = defineEmits<{
  result: [result: VoiceResult]
  /** A phrase that was heard but not used, for the voice history. */
  unclear: [result: VoiceResult]
}>()
const { settings } = storeToRefs(useKChessStore())
const state = ref<VoiceState>('off')
const error = ref('')
const errorKind = ref<VoiceErrorKind>('other')
const partial = ref('')
const heard = ref('')
const retry = ref('')
const level = ref(0)
const modelMessage = ref('Preparing voice input…')
/** Hold-to-speak is a saved preference, offered only where a control allows it. */
const pushTalk = computed(() => !!props.allowPushTalk && !!settings.value?.voicePushToTalk)
let capture: VoiceCapture | undefined
const loading = computed(() => state.value === 'loading')
const { access, note, openSettings } = useMicrophoneAccess((value) => {
  // Coming back from System Settings with access granted: start again without another click.
  if (state.value === 'error' && errorKind.value === 'denied' && value.status === 'granted')
    retryVoice()
})
const label = computed(() =>
  state.value === 'listening'
    ? 'Listening…'
    : state.value === 'loading'
      ? modelMessage.value
      : state.value === 'error'
        ? 'Voice input unavailable'
        : pushTalk.value && props.active
          ? 'Hold Space or the button to speak'
          : props.active
            ? 'Ready'
            : 'Ready for your next turn or run',
)
const icon = computed(() =>
  loading.value
    ? 'i-lucide-loader-circle'
    : state.value === 'error'
      ? 'i-lucide-mic-off'
      : 'i-lucide-mic',
)

/** Everything the compact bar has to say, most urgent first, on a single line. */
const line = computed(() => {
  if (state.value === 'error') return error.value
  if (partial.value) return `Hearing “${partial.value}”`
  if (retry.value || props.feedback) return retry.value || props.feedback!
  if (heard.value) return `Heard “${heard.value}”`
  if (state.value === 'loading' || !props.active) return label.value
  return pushTalk.value ? 'Hold Space to speak' : `Listening · ${props.hint}`
})
const lineTitle = computed(() =>
  [
    line.value,
    state.value === 'error' && errorKind.value === 'denied' && access.value?.launchedFromTerminal
      ? LAUNCHER_PERMISSION_HINT
      : '',
    state.value === 'error' ? note.value : '',
  ]
    .filter(Boolean)
    .join('\n'),
)

function newCapture(): VoiceCapture {
  modelMessage.value = 'Preparing voice input…'
  return new VoiceCapture({
    modelProgress: (progress) => {
      modelMessage.value =
        progress.phase === 'downloading'
          ? `Downloading voice model${progress.total ? ` · ${Math.min(100, Math.round((progress.received / progress.total) * 100))}%` : ' · about 40 MB'}…`
          : progress.phase === 'preparing'
            ? 'Preparing voice model…'
            : 'Loading voice model…'
    },
    state: (value) => {
      state.value = value
      if (value !== 'listening') level.value = 0
    },
    partial: (value) => {
      partial.value = value
    },
    error: (message, kind) => {
      error.value = message
      errorKind.value = kind
    },
    level: (value) => {
      level.value = value
    },
    result: (result) => {
      if (!enabled.value || !props.active) return
      const text = result.text
        .split(/\s+/)
        .filter((word) => word && word !== '[unk]')
        .join(' ')
      heard.value = text || result.text
      const cleaned: VoiceResult = { ...result, text, raw: result.text }
      const unclear = !text || result.confidence < 0.6
      if (unclear && !(props.acceptUnclear && text)) {
        retry.value = 'Didn’t catch that. Repeat, or try a phonetic name like “Echo four”.'
        emit('unclear', cleaned)
        return
      }
      retry.value = ''
      emit('result', { ...cleaned, unclear })
    },
  })
}

watch(
  enabled,
  (value) => {
    capture?.dispose()
    capture = undefined
    partial.value = heard.value = error.value = retry.value = ''
    if (value) {
      const session = newCapture()
      capture = session
      // Preflight even when paused, then release the mic until the run/turn begins.
      void session
        .prepare(props.grammar)
        .then(() => {
          if (capture === session) session.update(props.active, props.grammar, pushTalk.value)
        })
        .catch(() => {})
    }
  },
  { flush: 'sync', immediate: true },
)
watch(
  () => [props.active, props.contextKey, pushTalk.value] as const,
  () => {
    partial.value = ''
    capture?.update(props.active, props.grammar, pushTalk.value)
  },
  { flush: 'sync' },
)

async function prepare(): Promise<boolean> {
  if (!enabled.value) return true
  if (!capture || state.value === 'error') {
    capture?.dispose()
    capture = newCapture()
    error.value = ''
  }
  try {
    await capture.prepare(props.grammar)
    return true
  } catch {
    return false
  }
}
defineExpose({ prepare, loading })

function keydown(event: KeyboardEvent): void {
  const target = event.target as HTMLElement | null
  if (
    event.code !== 'Space' ||
    event.repeat ||
    event.metaKey ||
    event.ctrlKey ||
    event.altKey ||
    target?.closest('input, textarea, select, button, [contenteditable="true"]') ||
    !enabled.value ||
    !pushTalk.value ||
    !props.active
  )
    return
  event.preventDefault()
  capture?.talk(true)
}
function keyup(event: KeyboardEvent): void {
  if (event.code === 'Space') capture?.talk(false)
}
function blur(): void {
  capture?.talk(false)
}
function pointerdown(event: PointerEvent): void {
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  capture?.talk(true)
}
function retryVoice(): void {
  enabled.value = false
  enabled.value = true
}
onMounted(() => {
  window.addEventListener('keydown', keydown)
  window.addEventListener('keyup', keyup)
  window.addEventListener('blur', blur)
})
onBeforeUnmount(() => {
  capture?.dispose()
  window.removeEventListener('keydown', keydown)
  window.removeEventListener('keyup', keyup)
  window.removeEventListener('blur', blur)
})
</script>

<template>
  <div v-if="compact" class="voice-bar" :data-state="enabled ? state : 'off'">
    <UTooltip :text="enabled ? 'Turn off voice input' : 'Voice input: speak your moves'">
      <UButton
        size="sm"
        :variant="enabled ? 'soft' : 'ghost'"
        :color="enabled && state !== 'error' ? 'primary' : 'neutral'"
        :icon="enabled ? icon : 'i-lucide-mic'"
        :aria-pressed="enabled"
        aria-label="Voice input"
        :disabled="disabled"
        :ui="{ leadingIcon: loading ? 'animate-spin' : '' }"
        @click="enabled = !enabled"
        >{{ enabled ? '' : 'Voice' }}</UButton
      >
    </UTooltip>
    <template v-if="enabled">
      <span v-if="state === 'listening'" class="voice-meter" aria-hidden="true">
        <span :style="{ transform: `scaleX(${Math.max(0.04, level)})` }" />
      </span>
      <span
        class="voice-bar-text"
        :class="{ error: state === 'error' }"
        :title="lineTitle"
        role="status"
        aria-live="polite"
        >{{ line }}</span
      >
      <slot />
      <template v-if="state === 'error'">
        <UTooltip
          v-if="errorKind === 'denied' && access?.canOpenSettings"
          text="Open privacy settings"
        >
          <UButton
            size="xs"
            variant="ghost"
            color="neutral"
            icon="i-lucide-settings-2"
            aria-label="Open privacy settings"
            @click="openSettings"
          />
        </UTooltip>
        <UTooltip text="Try again">
          <UButton
            size="xs"
            variant="ghost"
            color="neutral"
            icon="i-lucide-rotate-ccw"
            aria-label="Try again"
            @click="retryVoice"
          />
        </UTooltip>
      </template>
      <UButton
        v-else-if="pushTalk && active && !loading"
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-mic"
        @pointerdown.prevent="pointerdown"
        @pointerup="capture?.talk(false)"
        @pointercancel="capture?.talk(false)"
        @keydown.space.prevent="capture?.talk(true)"
        @keyup.space.prevent="capture?.talk(false)"
        @blur="blur"
        >Hold</UButton
      >
    </template>
  </div>
  <div v-else class="voice-input form-stack">
    <USwitch
      v-model="enabled"
      label="Voice input"
      description="English · first use downloads about 40 MB, then works offline. Nothing is uploaded."
      :disabled="disabled"
    />
    <template v-if="enabled">
      <div class="voice-status" :data-state="state" role="status" aria-live="polite">
        <UIcon :name="icon" class="voice-status-icon" :class="{ 'animate-spin': loading }" />
        <span class="voice-status-label">{{ label }}</span>
        <span v-if="state === 'listening'" class="voice-meter" aria-hidden="true">
          <span :style="{ transform: `scaleX(${Math.max(0.04, level)})` }" />
        </span>
      </div>

      <div v-if="state === 'error'" class="voice-alert" role="alert">
        <p>{{ error }}</p>
        <p v-if="errorKind === 'denied' && access?.launchedFromTerminal" class="field-hint">
          {{ LAUNCHER_PERMISSION_HINT }}
        </p>
        <p v-if="note" class="field-hint">{{ note }}</p>
        <div class="voice-alert-actions">
          <UButton
            v-if="errorKind === 'denied' && access?.canOpenSettings"
            size="sm"
            icon="i-lucide-settings-2"
            @click="openSettings"
            >Open privacy settings</UButton
          >
          <UButton
            size="sm"
            color="neutral"
            variant="outline"
            icon="i-lucide-rotate-ccw"
            @click="retryVoice"
            >Try again</UButton
          >
        </div>
      </div>
      <p v-else class="field-hint">{{ hint }}</p>

      <UButton
        v-if="pushTalk && active && !loading && state !== 'error'"
        variant="outline"
        color="neutral"
        icon="i-lucide-mic"
        block
        @pointerdown.prevent="pointerdown"
        @pointerup="capture?.talk(false)"
        @pointercancel="capture?.talk(false)"
        @keydown.space.prevent="capture?.talk(true)"
        @keyup.space.prevent="capture?.talk(false)"
        @blur="blur"
        >Hold to speak</UButton
      >
      <p v-if="partial || heard" class="voice-heard" aria-live="off">
        <span>{{ partial ? 'Hearing' : 'Heard' }}</span> “{{ partial || heard }}”
      </p>
      <p v-if="retry || feedback" class="field-hint" role="status">{{ retry || feedback }}</p>
    </template>
  </div>
</template>

<style scoped>
.voice-bar {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  flex: 0 1 auto;
  min-width: 0;
  font-size: 12px;
  font-weight: 400;
}
.voice-bar-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--ui-text-muted);
}
.voice-bar[data-state='listening'] .voice-bar-text {
  color: var(--ui-text);
}
.voice-bar-text.error {
  color: var(--ui-error);
}
.voice-bar .voice-meter {
  width: 2.5rem;
}
.voice-status {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--ui-border);
  border-radius: 0.5rem;
  background: var(--ui-bg-elevated);
  font-size: 0.875rem;
  color: var(--ui-text-muted);
}
.voice-status[data-state='listening'] {
  border-color: var(--ui-primary);
  color: var(--ui-text);
}
.voice-status[data-state='listening'] .voice-status-icon {
  color: var(--ui-primary);
}
.voice-status[data-state='error'] {
  border-color: var(--ui-error);
  color: var(--ui-error);
}
.voice-status-icon {
  flex: none;
  width: 1rem;
  height: 1rem;
}
.voice-status-label {
  flex: 1 1 auto;
  min-width: 0;
}
.voice-meter {
  flex: none;
  width: 3.5rem;
  height: 0.375rem;
  border-radius: 999px;
  background: var(--ui-bg-accented);
  overflow: hidden;
}
.voice-meter > span {
  display: block;
  height: 100%;
  background: var(--ui-primary);
  transform-origin: left center;
  transition: transform 90ms linear;
}
.voice-alert {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  padding: 0.75rem;
  border-radius: 0.5rem;
  border: 1px solid color-mix(in oklab, var(--ui-error) 40%, transparent);
  background: color-mix(in oklab, var(--ui-error) 8%, transparent);
  font-size: 0.875rem;
}
.voice-alert-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
}
.voice-heard {
  font-size: 0.875rem;
  color: var(--ui-text);
}
.voice-heard > span {
  color: var(--ui-text-muted);
}
@media (prefers-reduced-motion: reduce) {
  .voice-meter > span {
    transition: none;
  }
}
</style>
