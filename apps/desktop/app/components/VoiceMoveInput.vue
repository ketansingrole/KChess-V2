<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { fen as fenOf } from '@kchess/rules/chess'
import type { Position } from '@kchess/rules/position'
import {
  MOVE_GRAMMAR,
  spokenChoice,
  spokenMove,
  type VoiceMoveChoice,
} from '@kchess/rules/voiceCommands'
import type { VoiceOutcome, VoiceSource } from '@kchess/contracts/types'
import type { VoiceResult } from '../utils/voiceCapture'
import { heardFields, logVoice, updateVoice } from '../utils/voiceLog'

/**
 * Spoken moves on a board, as a compact voice bar: a clear move is played (unless the player asks
 * to confirm moves), an unsure or ambiguous one waits for “confirm” or its number. Every phrase is
 * logged to the voice history with what came of it.
 */
const props = defineProps<{
  source: VoiceSource
  /** The position moves are spoken in; none while there is no board. */
  position: Position | undefined
  /** Moves are taken only while this is on (the player's turn at the live position). */
  active: boolean
  /** Anything else that makes a pending choice stale, such as the move list or shown ply. */
  contextKey?: string
  hint: string
  /** Plays a legal move; false when the board no longer accepts it. */
  play: (uci: string) => boolean
}>()
const enabled = defineModel<boolean>('enabled', { default: false })
const { settings } = storeToRefs(useKChessStore())

const feedback = ref('')
const choices = ref<VoiceMoveChoice[]>([])
const revision = ref(0)
const fen = computed(() => (props.position ? fenOf(props.position) : ''))
const context = computed(() => `${revision.value}:${fen.value}`)
const listening = computed(() => props.active && !!props.position)
let choiceContext = ''
let pendingAttempt: Promise<number | undefined> | undefined

function settle(outcome: VoiceOutcome, expected?: string): void {
  const attempt = pendingAttempt
  pendingAttempt = undefined
  if (attempt) void attempt.then((id) => updateVoice(id, { outcome, expected }))
}
watch(
  [() => props.position, () => props.contextKey, listening, enabled],
  () => {
    settle('abandoned')
    choices.value = []
    feedback.value = ''
    revision.value++
  },
  { flush: 'sync' },
)
function record(result: VoiceResult, outcome: VoiceOutcome, parsed?: string) {
  return logVoice({ source: props.source, ...heardFields(result), outcome, parsed, fen: fen.value })
}
function choose(choice: VoiceMoveChoice, outcome: 'played' | 'confirmed' | 'picked'): void {
  if (!enabled.value || !listening.value || choiceContext !== context.value) return
  // Settle before moving: the position watcher invalidates any remaining pending choices.
  settle(outcome, choice.san)
  choices.value = []
  feedback.value = props.play(choice.uci)
    ? `Played ${choice.san}.`
    : 'That move is no longer legal.'
}
function cancel(): void {
  settle('cancelled')
  choices.value = []
  feedback.value = 'Move cancelled.'
}
function hear(result: VoiceResult): void {
  const position = props.position
  if (!enabled.value || !listening.value || !position) return
  const index = spokenChoice(result.text)
  if (index && choices.value.length && !result.unclear) {
    void record(result, 'command', `#${index}`)
    const choice = choices.value[index - 1]
    if (choice) choose(choice, 'confirmed')
    else feedback.value = 'Choose one of the numbered moves.'
    return
  }
  const parsed = spokenMove(result.text, position)
  if (parsed.kind === 'cancel' && !result.unclear) {
    void record(result, 'command', 'cancel')
    cancel()
  } else if (parsed.kind === 'confirm' && !result.unclear) {
    void record(result, 'command', 'confirm')
    const only = choices.value[0]
    if (choices.value.length === 1 && only) choose(only, 'confirmed')
    else feedback.value = 'Say a move, or choose its number.'
  } else if (parsed.kind === 'move') {
    settle('replaced')
    choiceContext = context.value
    choices.value = parsed.choices
    pendingAttempt = record(result, 'pending', parsed.choices.map((choice) => choice.san).join('|'))
    const only = parsed.choices[0]
    if (
      parsed.choices.length === 1 &&
      only &&
      !settings.value?.voiceConfirmMoves &&
      !result.unclear
    )
      choose(only, 'played')
    else
      feedback.value =
        parsed.choices.length === 1
          ? `Play ${only?.san ?? 'that move'}? Say “confirm” or “cancel”.`
          : 'Several moves match: say its number, or select it.'
  } else {
    void record(result, result.unclear ? 'unclear' : 'invalid')
    feedback.value = result.unclear
      ? 'Didn’t catch that. Repeat, or try “Echo four”.'
      : `Heard “${result.text}”: no legal move matches.`
  }
}
function missed(result: VoiceResult): void {
  if (enabled.value && listening.value) void record(result, 'unclear')
}
onUnmounted(() => settle('abandoned'))
</script>

<template>
  <VoiceInput
    v-model:enabled="enabled"
    compact
    :active="listening"
    :context-key="context"
    :grammar="MOVE_GRAMMAR"
    :hint="hint"
    :feedback="feedback"
    allow-push-talk
    accept-unclear
    @result="hear"
    @unclear="missed"
  >
    <UButton
      v-for="(choice, index) in choices"
      :key="choice.uci"
      color="neutral"
      variant="outline"
      size="xs"
      class="shrink-0"
      @click="choose(choice, 'picked')"
      >{{ index + 1 }} · {{ choice.san }}</UButton
    >
    <UButton
      v-if="choices.length"
      color="neutral"
      variant="ghost"
      size="xs"
      icon="i-lucide-x"
      aria-label="Cancel move"
      @click="cancel"
    />
  </VoiceInput>
</template>
