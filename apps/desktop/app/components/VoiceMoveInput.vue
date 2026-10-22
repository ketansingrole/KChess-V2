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

/** What a page command made of a phrase: what was understood, and what to tell the player. */
export interface VoiceReply {
  said: string
  feedback: string
  /** False when the phrase was a command but not one that applies now. */
  understood?: boolean
}

/**
 * Spoken moves on a board, as a compact voice bar: a clear move is played (unless the player asks
 * to confirm moves), an unsure or ambiguous one waits for “confirm”, its number, or the move said
 * again. Pages add their own commands (“take back”, “back”) through `command`. Every phrase is
 * logged to the voice history with what came of it.
 */
const props = defineProps<{
  source: VoiceSource
  /** The position moves are spoken in; none while there is no board. */
  position: Position | undefined
  /** Moves are taken only while this is on (the player's turn at the live position). */
  active: boolean
  /** Keep listening while moves are not taken, for `command`. */
  listen?: boolean
  /** Why a spoken move is not taken now, while listening for commands. */
  idle?: string
  /** Anything else that makes a pending choice stale, such as the move list or shown ply. */
  contextKey?: string
  /**
   * The moves (SAN) that led to `position`: once the next one is played, by voice or by hand,
   * the phrases said here learn which move was meant.
   */
  line?: readonly string[]
  hint: string
  /** The words to listen for: moves, plus the page's commands. */
  grammar?: readonly string[]
  /** The page's own commands, tried before moves; undefined when the phrase is not one. */
  command?: (text: string) => VoiceReply | undefined
  /** Play a lone matching move at once, even an unsure one: trying a move here loses nothing. */
  immediate?: boolean
  /** Plays a legal move; false when the board did not keep it. */
  play: (uci: string) => boolean
}>()
const enabled = defineModel<boolean>('enabled', { default: false })
const { settings } = storeToRefs(useKChessStore())

const feedback = ref('')
const choices = ref<VoiceMoveChoice[]>([])
const revision = ref(0)
const fen = computed(() => (props.position ? fenOf(props.position) : ''))
const context = computed(() => `${revision.value}:${fen.value}`)
const taking = computed(() => props.active && !!props.position)
const listening = computed(() => (props.active || props.listen) && !!props.position)
let choiceContext = ''

/*
 * Phrases said in one position are kept together so that, once a move is made there, each learns
 * what was meant: that pairing of "heard" and "meant" shows where recognition goes wrong.
 */
type Entry = Promise<number | undefined>
let pendingAttempt: Entry | undefined
let lastMiss: Entry | undefined
let positionEntries: Entry[] = []
let positionLine: readonly string[] = []

/** `track`: the phrase was about this position's move, so it learns which move was then played. */
function record(result: VoiceResult, outcome: VoiceOutcome, parsed?: string, track = true): Entry {
  const retryOf = lastMiss
  const entry = (async () =>
    logVoice({
      source: props.source,
      ...heardFields(result),
      outcome,
      parsed,
      fen: fen.value,
      retryOf: (await retryOf) ?? undefined,
    }))()
  if (!track) return entry
  if (!positionEntries.length) positionLine = [...(props.line ?? [])]
  positionEntries.push(entry)
  if (outcome === 'invalid' || outcome === 'unclear') lastMiss = entry
  return entry
}
/** Close the pending choice with how it ended. */
function settle(outcome: VoiceOutcome, expected?: string): void {
  const attempt = pendingAttempt
  pendingAttempt = undefined
  if (!attempt) return
  if (outcome !== 'played' && outcome !== 'confirmed' && outcome !== 'picked') lastMiss = attempt
  void attempt.then((id) => updateVoice(id, { outcome, expected }))
}
/** The position moved on: tell its phrases which move was played there. */
function learnMeaning(): void {
  const entries = positionEntries
  const before = positionLine
  positionEntries = []
  lastMiss = undefined
  const line = props.line
  if (!entries.length || !line) return
  const played = line.length === before.length + 1 && before.every((san, i) => line[i] === san)
  const san = played ? line[before.length] : undefined
  if (san) for (const entry of entries) void entry.then((id) => updateVoice(id, { expected: san }))
}

watch(
  [() => props.position, () => props.contextKey, taking, listening, enabled],
  () => {
    if (choices.value.length) {
      settle('abandoned')
      choices.value = []
      feedback.value = ''
    }
    revision.value++
  },
  { flush: 'sync' },
)
// Not synchronous: `line` is updated after `position` when both change.
watch([() => props.position, () => props.contextKey], learnMeaning)

function choose(choice: VoiceMoveChoice, outcome: 'played' | 'confirmed' | 'picked'): void {
  if (!enabled.value || !taking.value || choiceContext !== context.value) return
  // Settle before moving: the position watcher abandons any choice still pending.
  settle(outcome, choice.san)
  choices.value = []
  feedback.value = props.play(choice.uci) ? `Played ${choice.san}.` : `Couldn’t play ${choice.san}.`
}
function cancel(): void {
  settle('cancelled')
  choices.value = []
  feedback.value = 'Move cancelled.'
}
function offer(result: VoiceResult, offered: VoiceMoveChoice[]): void {
  const sans = offered.map((choice) => choice.san).join('|')
  const only = offered.length === 1 ? offered[0] : undefined
  // Saying the pending move again confirms it, which is easier than a separate word.
  const pending = choices.value.length === 1 ? choices.value[0] : undefined
  if (only && pending && only.uci === pending.uci) {
    void record(result, 'command', `repeat ${sans}`)
    return choose(pending, 'confirmed')
  }
  if (choices.value.length) settle('replaced')
  choiceContext = context.value
  choices.value = offered
  // An unsure hearing is never played blind, unless trying a move here costs nothing.
  if (only && (props.immediate || (!settings.value?.voiceConfirmMoves && !result.unclear))) {
    void record(result, 'played', sans)
    return choose(only, 'played')
  }
  pendingAttempt = record(result, 'pending', sans)
  feedback.value = !only
    ? 'Several moves match: say its number, or select it.'
    : result.unclear
      ? `Did you mean ${only.san}? Say “confirm”, or repeat the move.`
      : `Play ${only.san}? Say “confirm” or “cancel”.`
}
function hear(result: VoiceResult): void {
  const position = props.position
  if (!enabled.value || !listening.value || !position) return
  const reply = props.command?.(result.text)
  if (reply) {
    void record(result, reply.understood === false ? 'invalid' : 'command', reply.said, false)
    // Drop a pending move now, so that the command's own answer stays on screen.
    if (choices.value.length) settle('abandoned')
    choices.value = []
    feedback.value = reply.feedback
    return
  }
  if (!taking.value) {
    void record(result, 'invalid', 'not your move', false)
    feedback.value = props.idle ?? ''
    return
  }
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
    offer(result, parsed.choices)
  } else {
    void record(result, result.unclear ? 'unclear' : 'invalid')
    feedback.value = result.unclear
      ? 'Didn’t catch that. Repeat, or try “Echo four” for E4.'
      : `Heard “${result.text}”: no legal move matches.`
  }
}
/** Heard, but too unsure (or only noise) to act on. */
function missed(result: VoiceResult): void {
  if (enabled.value && listening.value) void record(result, 'unclear', undefined, taking.value)
}
onUnmounted(() => settle('abandoned'))
</script>

<template>
  <VoiceInput
    v-model:enabled="enabled"
    compact
    :active="listening"
    :context-key="context"
    :grammar="grammar ?? MOVE_GRAMMAR"
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
