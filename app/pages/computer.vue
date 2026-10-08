<script setup lang="ts">
import { useGameArchiveStore } from '../stores/gameArchive'
import { computed, onMounted, ref, watch } from 'vue'
import type { PlayerInfo } from '../components/PlayerLine.vue'
import {
  GAME_GRAMMAR,
  spokenChoice,
  spokenCommand,
  spokenMove,
  type GameCommand,
  type VoiceMoveChoice,
} from '../../src/shared/voiceCommands'
import { setupPgn } from '../../src/shared/chess'
import { VARIANT_LABELS } from '../../src/shared/variant'
import { useAnalysisStore } from '../stores/analysis'
import type { VoiceResult } from '../utils/voiceCapture'
import { heardFields, logVoice, updateVoice } from '../utils/voiceLog'
import type { VoiceOutcome } from '../../src/shared/types'
import { DEFAULT_ENGINE_LEVELS, engineLevelLabel } from '../../src/shared/engineLevels'

const archive = useGameArchiveStore()
const store = useKChessStore()
const {
  localMoves,
  localGameEpoch,
  localPly,
  level,
  userColor,
  thinking,
  engineReady,
  engineChecking,
  localDisplay,
  localHistory,
  localLast,
  localDests,
  localTurn,
  localCheck,
  localInteractive,
  localCanPlay,
  localResult,
  localGame,
  localSetup,
  zenActive,
  settings,
} = storeToRefs(store)
const { newGame, takeback, resign, makeMove, selectPage, fen, recheckEngine } = store

// The engine can be removed or replaced while the app is open; look again whenever this page opens.
onMounted(() => void recheckEngine())

/** Only the levels the player keeps in Settings → Gameplay. */
const levels = computed(() =>
  (settings.value?.engineLevels ?? DEFAULT_ENGINE_LEVELS).map((id) => ({
    label: engineLevelLabel(id),
    value: id,
  })),
)
const otherColor = computed(() => (userColor.value === 'white' ? 'black' : 'white'))

/** A game with moves that has not finished: leaving it needs a confirmation. */
const inProgress = computed(() => localMoves.value.length > 0 && !localResult.value)
const confirmNew = ref(false)
const confirmResign = ref(false)
const pendingColor = ref<'white' | 'black' | null>(null)
/** A yes/no question is on screen; “confirm” and “cancel” answer it. */
const dialogOpen = computed(() => confirmNew.value || confirmResign.value || !!pendingColor.value)
const voiceEnabled = ref(false)
const voiceFeedback = ref('')
const voiceChoices = ref<VoiceMoveChoice[]>([])
let choiceContext = ''
const voiceContext = computed(
  () => `${localGameEpoch.value}:${userColor.value}:${localMoves.value.join(' ')}`,
)
/** Spoken moves are taken only on your turn… */
const voiceActive = computed(() => localInteractive.value && !thinking.value && !dialogOpen.value)
/** …but game commands (“take back”, “new game”) work whenever the engine is there to play. */
const voiceListening = computed(() => engineReady.value)
/*
 * Voice history: every phrase is logged with what came of it. Phrases said in one position are
 * kept together so that, once the player's move is made (by voice or by hand), each learns what
 * was meant — that pairing of "heard" and "meant" is what shows where recognition goes wrong.
 */
type LogEntry = Promise<number | undefined>
let pendingEntry: LogEntry | undefined
let lastMiss: LogEntry | undefined
let positionEntries: LogEntry[] = []
let positionMoves: string[] = []

/** `track`: the phrase was about this position's move, so it learns which move was then played. */
function record(
  result: VoiceResult,
  outcome: VoiceOutcome,
  parsed?: string,
  track = true,
): LogEntry {
  const retryOf = lastMiss
  const entry = (async () =>
    logVoice({
      source: 'computer',
      ...heardFields(result),
      outcome,
      parsed,
      fen: fen(localGame.value),
      retryOf: (await retryOf) ?? undefined,
    }))()
  if (!track) return entry
  if (!positionEntries.length) positionMoves = [...localMoves.value]
  positionEntries.push(entry)
  if (['invalid', 'unclear'].includes(outcome)) lastMiss = entry
  return entry
}
/** Close the pending choice with how it ended. */
function settle(outcome: VoiceOutcome): void {
  const entry = pendingEntry
  pendingEntry = undefined
  if (!entry) return
  if (outcome !== 'confirmed' && outcome !== 'picked') lastMiss = entry
  void entry.then((id) => updateVoice(id, { outcome }))
}
/** The position moved on: tell this position's phrases which move the player actually chose. */
function learnMeaning(): void {
  const entries = positionEntries
  const before = positionMoves
  positionEntries = []
  lastMiss = undefined
  if (!entries.length) return
  const moves = localMoves.value
  const played = moves.length === before.length + 1 && before.every((m, i) => moves[i] === m)
  const san = played ? localHistory.value[before.length] : undefined
  if (san) for (const entry of entries) void entry.then((id) => updateVoice(id, { expected: san }))
}

watch(
  [voiceContext, voiceActive, voiceEnabled],
  ([context], [previous]) => {
    if (voiceChoices.value.length) settle('abandoned')
    voiceChoices.value = []
    choiceContext = ''
    if (context !== previous) learnMeaning()
  },
  { flush: 'sync' },
)

function playVoice(choice: VoiceMoveChoice, how: 'confirmed' | 'picked' = 'confirmed'): void {
  if (!voiceActive.value || !voiceEnabled.value || choiceContext !== voiceContext.value) return
  settle(how)
  voiceChoices.value = []
  voiceFeedback.value = `Played ${choice.san}.`
  makeMove(choice.uci)
}
function cancelVoice(): void {
  settle('cancelled')
  voiceChoices.value = []
  voiceFeedback.value = 'Move cancelled.'
}
/** Heard, but too unsure (or only noise) to act on. */
function missedMove(result: VoiceResult): void {
  if (voiceEnabled.value) void record(result, 'unclear', undefined, voiceActive.value)
}

const COMMAND_NAMES: Record<GameCommand, string> = {
  takeback: 'take back',
  newGame: 'new game',
  resign: 'resign',
  flip: 'switch sides',
}
/** Game controls by voice; anything that throws the game away asks first, like its button. */
function runCommand(command: GameCommand, result: VoiceResult): void {
  void record(result, 'command', COMMAND_NAMES[command], false)
  if (command === 'takeback') {
    if (localMoves.value.length < 2) voiceFeedback.value = 'Nothing to take back yet.'
    else {
      takeback()
      voiceFeedback.value = 'Took back your last move.'
    }
  } else if (command === 'newGame') {
    requestNewGame()
    voiceFeedback.value = confirmNew.value
      ? 'Start a new game? Say “confirm” or “cancel”.'
      : 'New game started.'
  } else if (command === 'resign') {
    if (!inProgress.value) voiceFeedback.value = 'There’s no game in progress to resign.'
    else {
      confirmResign.value = true
      voiceFeedback.value = 'Resign this game? Say “confirm” or “cancel”.'
    }
  } else {
    switchSides()
    voiceFeedback.value = pendingColor.value
      ? 'Switch sides and restart? Say “confirm” or “cancel”.'
      : `You now play ${userColor.value}.`
  }
}
/** Answer the open yes/no question. */
function answerDialog(result: VoiceResult): void {
  const answer = spokenMove(result.text, localGame.value).kind
  if (answer !== 'confirm' && answer !== 'cancel') {
    void record(result, 'invalid', 'expected confirm or cancel', false)
    voiceFeedback.value = 'Say “confirm” or “cancel”.'
    return
  }
  void record(result, 'command', answer, false)
  if (answer === 'cancel') {
    confirmNew.value = confirmResign.value = false
    pendingColor.value = null
    voiceFeedback.value = 'Cancelled.'
  } else if (confirmResign.value) {
    confirmResign.value = false
    resign()
    voiceFeedback.value = 'You resigned. Say “new game” to play again.'
  } else if (confirmNew.value) {
    confirmNew.value = false
    newGame()
    voiceFeedback.value = 'New game started.'
  } else {
    confirmColorChange()
    voiceFeedback.value = `New game: you play ${userColor.value}.`
  }
}

function hearMove(result: VoiceResult): void {
  if (!voiceEnabled.value) return
  if (dialogOpen.value) return answerDialog(result)
  const command = spokenCommand(result.text)
  if (command) return runCommand(command, result)
  if (!voiceActive.value) {
    void record(result, 'invalid', 'not your move', false)
    voiceFeedback.value = localResult.value
      ? 'The game is over. Say “new game” to play again.'
      : localPly.value !== localMoves.value.length
        ? 'Go to the last move to keep playing.'
        : 'Wait for Stockfish, then say your move.'
    return
  }
  const index = spokenChoice(result.text)
  if (index && voiceChoices.value.length) {
    const choice = voiceChoices.value[index - 1]
    void record(result, 'command', `#${index}`)
    if (choice) playVoice(choice)
    else voiceFeedback.value = 'Choose one of the numbered moves.'
    return
  }
  const parsed = spokenMove(result.text, localGame.value)
  if (parsed.kind === 'cancel') {
    void record(result, 'command', 'cancel')
    cancelVoice()
  } else if (parsed.kind === 'confirm') {
    void record(result, 'command', 'confirm')
    if (voiceChoices.value.length === 1) playVoice(voiceChoices.value[0]!)
    else voiceFeedback.value = 'Say a move, or choose its number.'
  } else if (parsed.kind === 'invalid') {
    if (voiceChoices.value.length) settle('replaced')
    voiceChoices.value = []
    void record(result, result.unclear ? 'unclear' : 'invalid')
    voiceFeedback.value = result.unclear
      ? 'Didn’t catch that. Repeat, or try “Echo four” for E4.'
      : `Heard “${result.text}”: no legal move matches. Try “E two to E four” or “Knight F three”.`
  } else {
    const sans = parsed.choices.map((choice) => choice.san).join('|')
    // Repeating the pending move confirms it, which is easier than a separate word.
    const pending = voiceChoices.value
    if (
      pending.length === 1 &&
      parsed.choices.length === 1 &&
      parsed.choices[0]!.uci === pending[0]!.uci
    ) {
      void record(result, 'command', `repeat ${sans}`)
      playVoice(pending[0]!)
      return
    }
    if (pending.length) settle('replaced')
    choiceContext = voiceContext.value
    voiceChoices.value = parsed.choices
    // An unsure hearing is never played blind: it always waits for a confirmation.
    if (parsed.choices.length === 1 && !settings.value?.voiceConfirmMoves && !result.unclear) {
      void record(result, 'played', sans)
      playVoice(parsed.choices[0]!)
      return
    }
    pendingEntry = record(result, 'pending', sans)
    voiceFeedback.value =
      parsed.choices.length > 1
        ? 'Choose a move by saying its number, or select it.'
        : result.unclear
          ? `Did you mean ${parsed.choices[0]!.san}? Say “confirm”, or repeat the move.`
          : `Say “confirm” to play ${parsed.choices[0]!.san}, or “cancel”.`
  }
}
const analysis = useAnalysisStore()
/** Review the game on the analysis board, opened at the move being looked at. */
function reviewGame(): void {
  if (!canReview.value) return
  if (!analysis.loadPgn(setupPgn(localSetup.value, localMoves.value), localPly.value)) return
  analysis.orientation = userColor.value
  const computer = `Stockfish (${engineLevelLabel(level.value)})`
  analysis.origin = {
    white: userColor.value === 'white' ? 'You' : computer,
    black: userColor.value === 'black' ? 'You' : computer,
  }
  void analysis.requestReview()
  selectPage('analysis')
}

/** The analysis board and reviews play standard rules (from any position). */
const canReview = computed(() => localSetup.value.variant === 'standard')
const setupLabel = computed(() =>
  localSetup.value.variant !== 'standard'
    ? VARIANT_LABELS[localSetup.value.variant]
    : localSetup.value.fen.startsWith('rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq')
      ? ''
      : 'From position',
)
const optionsOpen = ref(false)
/** Whoever sits at the bottom of the board is you, so flipping it switches sides. */
const orientation = computed(() => userColor.value)
const confirmColor = computed({
  get: () => pendingColor.value !== null,
  set: (open: boolean) => {
    if (!open) pendingColor.value = null
  },
})

function requestNewGame(): void {
  if (inProgress.value) confirmNew.value = true
  else newGame()
}
function switchSides(): void {
  // Before the first move there is nothing to lose; otherwise ask, since it starts a new game.
  if (localMoves.value.length) pendingColor.value = otherColor.value
  else applyColor(otherColor.value)
}
function applyColor(value: 'white' | 'black'): void {
  userColor.value = value
  newGame()
}
function confirmColorChange(): void {
  if (pendingColor.value) applyColor(pendingColor.value)
  pendingColor.value = null
}

const viewingHistory = computed(() => localPly.value !== localMoves.value.length)
const banner = computed<{
  kind: '' | 'win' | 'loss' | 'alert'
  icon: string
  title: string
  detail?: string
  spin?: boolean
}>(() => {
  if (!engineReady.value)
    return {
      kind: 'alert',
      icon: 'i-lucide-triangle-alert',
      title: 'Stockfish not found',
      detail: 'Install or choose an engine in Settings to play.',
    }
  const result = localResult.value
  if (result)
    return {
      kind: result.kind === 'draw' ? '' : result.kind,
      icon:
        result.kind === 'win'
          ? 'i-lucide-trophy'
          : result.kind === 'loss'
            ? 'i-lucide-flag'
            : 'i-lucide-handshake',
      title: result.title,
      detail: result.detail,
    }
  if (viewingHistory.value)
    return {
      kind: '',
      icon: 'i-lucide-history',
      title: 'Reviewing an earlier position',
      detail: 'Go to the last move to keep playing.',
    }
  if (thinking.value)
    return { kind: '', icon: 'i-lucide-loader-circle', spin: true, title: 'Stockfish is thinking…' }
  const yours = localTurn.value === userColor.value
  return {
    kind: '',
    icon: yours ? 'i-lucide-mouse-pointer-click' : 'i-lucide-hourglass',
    title: yours ? 'Your move' : "Stockfish's move",
    detail: localCheck.value
      ? 'Check!'
      : `${localTurn.value === 'white' ? 'White' : 'Black'} to play`,
  }
})

/** You are always at the bottom; Stockfish faces you. */
const players = computed<{ top: PlayerInfo; bottom: PlayerInfo }>(() => {
  const computerPlayer: PlayerInfo = {
    name: 'Stockfish',
    icon: 'i-lucide-cpu',
    status: thinking.value ? 'Thinking…' : undefined,
    busy: thinking.value,
    presence: engineChecking.value
      ? { state: 'checking', label: 'Checking…' }
      : engineReady.value
        ? { state: 'online', label: 'Live' }
        : { state: 'unavailable', label: 'Unavailable' },
  }
  const other = userColor.value === 'white' ? 'black' : 'white'
  computerPlayer.clock = store.computerClockText(other)
  return {
    top: computerPlayer,
    bottom: { name: 'You', icon: 'i-lucide-user', clock: store.computerClockText(userColor.value) },
  }
})
</script>

<template>
  <div>
    <p v-if="archive.error" role="alert" class="p-3 text-error">
      History saving: {{ archive.error }}
    </p>
    <p v-if="store.localSaveError" role="alert" class="p-3 text-error">
      Automatic saving: {{ store.localSaveError }}
    </p>

    <PageHeader title="Play with Computer" />

    <div class="play-layout">
      <PlayBoard
        :fen="fen(localDisplay)"
        :orientation="orientation"
        :theme="settings.boardTheme"
        :coordinates="settings.coordinates"
        :piece-set="settings.pieceSet"
        :animation="settings.pieceAnimation"
        :movable="localInteractive"
        :interactive="localCanPlay"
        :movable-color="userColor"
        :premove="settings.premove"
        :show-dests="settings.showLegalMoves"
        :promotion="settings.promotion"
        :dests="localInteractive ? localDests : undefined"
        :last-move="localLast"
        :check="localCheck"
        :turn-color="localTurn"
        :live="!localResult"
        :top="players.top"
        :bottom="players.bottom"
        :blindfold="settings.blindfold && !localResult"
        :variant="localSetup.variant"
        @move="makeMove"
      >
        <template #top-name>
          <span class="player-name">Stockfish</span>
          <USelect
            v-model="level"
            :items="levels"
            aria-label="Difficulty"
            size="xs"
            variant="soft"
            icon="i-lucide-gauge"
            class="level-select"
            :ui="{
              content:
                'w-max min-w-(--reka-select-trigger-width) max-h-[min(28rem,var(--reka-select-content-available-height,28rem))]',
            }"
          />
        </template>
        <template #bottom-aside>
          <VoiceInput
            v-model:enabled="voiceEnabled"
            compact
            :active="voiceListening"
            :context-key="voiceContext"
            :grammar="GAME_GRAMMAR"
            hint="Say “E four”, “Knight F three”, or “take back”."
            :feedback="voiceFeedback"
            allow-push-talk
            accept-unclear
            @result="hearMove"
            @unclear="missedMove"
          >
            <template v-if="voiceChoices.length">
              <UButton
                v-for="(choice, index) in voiceChoices"
                :key="choice.uci"
                color="neutral"
                variant="outline"
                size="xs"
                class="shrink-0"
                @click="playVoice(choice, 'picked')"
                >{{ index + 1 }} · {{ choice.san }}</UButton
              >
              <UTooltip text="Cancel move">
                <UButton
                  color="neutral"
                  variant="ghost"
                  size="xs"
                  icon="i-lucide-x"
                  aria-label="Cancel move"
                  @click="cancelVoice"
                />
              </UTooltip>
            </template>
          </VoiceInput>
        </template>
      </PlayBoard>
      <MovePanel
        :moves="localHistory"
        :ply="localPly"
        :empty-text="
          userColor === 'white'
            ? 'Make your move to start. Flip the board to play Black.'
            : 'You play Black. Flip the board to play White.'
        "
        :flip-label="`Switch sides (play ${otherColor})`"
        @select="localPly = $event"
        @flip="switchSides"
      >
        <template #top>
          <div class="flex flex-wrap items-center gap-2 text-xs muted">
            <span v-if="setupLabel" class="font-semibold">{{ setupLabel }}</span>
            <span v-if="store.localClock"
              >{{ store.localClock.minutes }}+{{ store.localClock.increment }}</span
            >
            <span class="flex-1" />
            <UButton
              size="xs"
              variant="ghost"
              color="neutral"
              icon="i-lucide-settings-2"
              @click="optionsOpen = true"
              >Game options</UButton
            >
            <UTooltip text="Blindfold: hide the pieces">
              <UButton
                size="xs"
                :variant="settings.blindfold ? 'soft' : 'ghost'"
                :color="settings.blindfold ? 'primary' : 'neutral'"
                icon="i-lucide-eye-off"
                aria-label="Blindfold"
                :aria-pressed="settings.blindfold"
                @click="store.toggleSetting('blindfold')"
              />
            </UTooltip>
            <UTooltip text="Zen mode (Z)">
              <UButton
                size="xs"
                :variant="zenActive ? 'soft' : 'ghost'"
                :color="zenActive ? 'primary' : 'neutral'"
                icon="i-lucide-maximize"
                aria-label="Zen mode"
                :aria-pressed="settings.zenMode"
                @click="store.toggleSetting('zenMode')"
              />
            </UTooltip>
          </div>
          <OpeningName
            v-if="localSetup.variant === 'standard'"
            :setup="localSetup"
            :moves="localMoves"
            :ply="localPly"
          />
          <div class="status-banner" :class="banner.kind" :role="localResult ? 'alert' : 'status'">
            <UIcon
              :name="banner.icon"
              class="status-icon"
              :class="{ 'animate-spin': banner.spin }"
            />
            <div class="banner-text">
              <strong>{{ banner.title }}</strong>
              <span v-if="banner.detail" class="detail">{{ banner.detail }}</span>
              <UButton
                v-if="viewingHistory && !localResult"
                size="xs"
                variant="link"
                color="neutral"
                class="px-0"
                @click="localPly = localMoves.length"
                >Back to game</UButton
              >
              <div v-if="!engineReady" class="flex flex-wrap gap-x-3">
                <UButton
                  size="xs"
                  variant="link"
                  color="neutral"
                  class="px-0"
                  :loading="engineChecking"
                  @click="recheckEngine"
                  >Check again</UButton
                >
                <UButton
                  size="xs"
                  variant="link"
                  color="neutral"
                  class="px-0"
                  @click="selectPage('settings')"
                  >Set up Stockfish</UButton
                >
              </div>
            </div>
            <UButton
              v-if="localResult && localMoves.length && canReview"
              class="banner-action"
              size="sm"
              variant="soft"
              color="neutral"
              icon="i-lucide-sparkles"
              @click="reviewGame"
              >Review game</UButton
            >
          </div>
        </template>
        <template #bottom>
          <div class="panel-actions panel-divider pt-3.5">
            <UButton
              variant="outline"
              color="neutral"
              icon="i-lucide-undo-2"
              :disabled="localMoves.length < 2"
              @click="takeback"
              >Take back</UButton
            >
            <UButton
              v-if="inProgress"
              variant="outline"
              color="neutral"
              icon="i-lucide-flag"
              @click="confirmResign = true"
              >Resign</UButton
            >
            <UButton
              :variant="localResult ? 'solid' : 'outline'"
              :color="localResult ? 'primary' : 'neutral'"
              icon="i-lucide-rotate-ccw"
              @click="requestNewGame"
              >New game</UButton
            >
          </div>
          <div class="panel-actions zen-hide">
            <ExportGame
              :setup="localSetup"
              :moves="localMoves"
              :orientation="userColor"
              :white="userColor === 'white' ? 'You' : 'Stockfish'"
              :black="userColor === 'black' ? 'You' : 'Stockfish'"
              :ply="localPly"
            />
          </div>
        </template>
      </MovePanel>
    </div>

    <ConfirmDialog
      v-model:open="confirmNew"
      title="Start a new game?"
      description="Your current game is still in progress and will be lost."
      confirm-label="New game"
      @confirm="newGame()"
    />
    <ComputerGameOptions v-model:open="optionsOpen" />
    <ConfirmDialog
      v-model:open="confirmResign"
      title="Resign this game?"
      description="Stockfish wins. The game stays on the board so you can go through it."
      confirm-label="Resign"
      @confirm="resign"
    />
    <ConfirmDialog
      v-model:open="confirmColor"
      :title="`Play as ${otherColor === 'white' ? 'White' : 'Black'}?`"
      :description="
        inProgress
          ? 'Your current game is still in progress. Switching sides starts a new game.'
          : 'Switching sides starts a new game.'
      "
      confirm-label="Switch and restart"
      @confirm="confirmColorChange"
    />
  </div>
</template>

<style scoped>
.play-layout {
  /* No visible page header here, and the move box shares the player's line: the board takes that height. */
  --board-chrome: 206px;
}
.level-select {
  min-width: 13rem;
}
</style>
