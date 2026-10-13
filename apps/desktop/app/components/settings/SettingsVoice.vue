<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { useKChessStore } from '../../stores/kchess'
import { LAUNCHER_PERMISSION_HINT, useMicrophoneAccess } from '../../utils/microphone'
import { describeMicError } from '../../utils/voiceCapture'
import { micBadges } from './options'

const store = useKChessStore()
const { settings } = storeToRefs(store)

const mic = useMicrophoneAccess()
const micBadge = computed(() => micBadges[mic.access.value?.status ?? 'unknown'])
const micHint = computed(() => {
  switch (mic.access.value?.status) {
    case 'granted':
      return 'KChess can use your microphone for voice input.'
    case 'denied':
      return 'Access was turned off. Allow KChess in your system privacy settings.'
    case 'restricted':
      return 'A device policy blocks the microphone for this app.'
    case 'not-determined':
      return 'Your system will ask the first time voice input starts.'
    default:
      return 'Voice input asks for access when you turn it on.'
  }
})
const micAsking = ref(false)
async function allowMicrophone(): Promise<void> {
  micAsking.value = true
  try {
    await mic.refresh(true)
  } finally {
    micAsking.value = false
  }
}

/** A short live level check: opens the mic, shows its loudness, and releases it when stopped. */
const micTest = ref<{ stream: MediaStream; context: AudioContext; frame: number } | null>(null)
const micLevel = ref(0)
const micDevice = ref('')
const micTestError = ref('')
async function toggleMicTest(): Promise<void> {
  if (micTest.value) return stopMicTest()
  micTestError.value = ''
  try {
    const access = await mic.refresh(true)
    if (access?.status === 'denied' || access?.status === 'restricted')
      throw new DOMException('denied', 'NotAllowedError')
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true, video: false })
    const context = new AudioContext()
    const analyser = context.createAnalyser()
    analyser.fftSize = 1024
    context.createMediaStreamSource(stream).connect(analyser)
    const samples = new Float32Array(analyser.fftSize)
    micDevice.value = stream.getAudioTracks()[0]?.label ?? ''
    const tick = (): void => {
      if (!micTest.value) return
      analyser.getFloatTimeDomainData(samples)
      let sum = 0
      for (const sample of samples) sum += sample * sample
      micLevel.value = Math.min(1, Math.sqrt(sum / samples.length) * 5)
      micTest.value.frame = requestAnimationFrame(tick)
    }
    micTest.value = { stream, context, frame: 0 }
    tick()
    await mic.refresh()
  } catch (cause) {
    console.warn('[settings] Microphone test failed:', cause)
    micTestError.value = describeMicError(cause).message
    await mic.refresh()
  }
}
function stopMicTest(): void {
  const test = micTest.value
  micTest.value = null
  micLevel.value = 0
  if (!test) return
  cancelAnimationFrame(test.frame)
  test.stream.getTracks().forEach((track) => track.stop())
  void test.context.close().catch((error: unknown) => {
    console.warn('[settings] Microphone test cleanup failed:', error)
  })
}
onBeforeUnmount(stopMicTest)
</script>

<template>
  <section id="settings-voice" class="card settings-list" aria-labelledby="voice-title">
    <h2 id="voice-title" class="sr-only">Voice input</h2>
    <div class="setting-row">
      <div class="setting-info">
        <span class="setting-title flex items-center gap-2"
          >Microphone access
          <UBadge :color="micBadge.color" variant="subtle" size="sm">{{
            micBadge.label
          }}</UBadge></span
        >
        <span v-if="mic.access.value?.status !== 'granted'" class="setting-hint">{{
          micHint
        }}</span>
        <span
          v-if="mic.access.value?.launchedFromTerminal && mic.access.value.status !== 'granted'"
          class="setting-hint"
          >{{ LAUNCHER_PERMISSION_HINT }}</span
        >
        <span v-if="mic.note.value" class="setting-hint">{{ mic.note.value }}</span>
      </div>
      <div class="friend-actions">
        <UButton
          v-if="mic.access.value?.status === 'not-determined'"
          icon="i-lucide-mic"
          :loading="micAsking"
          @click="allowMicrophone"
          >Allow microphone</UButton
        >
        <UButton
          v-if="mic.access.value?.canOpenSettings"
          variant="ghost"
          color="neutral"
          icon="i-lucide-settings-2"
          @click="mic.openSettings"
          >Privacy settings</UButton
        >
      </div>
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span class="setting-title">Test your microphone</span>
        <span v-if="micTest || micTestError" class="setting-hint" role="status">{{
          micTestError || (micTest ? `Speak now${micDevice ? ` · ${micDevice}` : ''}` : '')
        }}</span>
        <span v-if="micTest" class="mic-meter" aria-hidden="true">
          <span :style="{ transform: `scaleX(${Math.max(0.02, micLevel)})` }" />
        </span>
      </div>
      <div class="friend-actions">
        <UButton
          variant="outline"
          color="neutral"
          :icon="micTest ? 'i-lucide-square' : 'i-lucide-audio-lines'"
          @click="toggleMicTest"
          >{{ micTest ? 'Stop' : 'Start test' }}</UButton
        >
      </div>
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="voice-confirm-label" class="setting-title">Confirm spoken moves</span>
        <span class="setting-hint">Say “confirm” before playing the move.</span>
      </div>
      <USwitch
        v-model="settings.voiceConfirmMoves"
        aria-labelledby="voice-confirm-label"
        class="setting-switch"
      />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="voice-ptt-label" class="setting-title">Hold to speak</span>
        <span class="setting-hint">Hold Space or the on-screen button.</span>
      </div>
      <USwitch
        v-model="settings.voicePushToTalk"
        aria-labelledby="voice-ptt-label"
        class="setting-switch"
      />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="voice-history-label" class="setting-title">Keep voice history</span>
        <span class="setting-hint">Saves recognized text on this device; no audio.</span>
      </div>
      <USwitch
        v-model="settings.voiceHistory"
        aria-labelledby="voice-history-label"
        class="setting-switch"
      />
    </div>
  </section>
</template>

<style scoped>
.mic-meter {
  display: block;
  width: min(12rem, 100%);
  height: 0.375rem;
  margin-top: 0.375rem;
  border-radius: 999px;
  background: var(--ui-bg-accented);
  overflow: hidden;
}
.mic-meter > span {
  display: block;
  height: 100%;
  background: var(--ui-primary);
  transform-origin: left center;
  transition: transform 80ms linear;
}
</style>
