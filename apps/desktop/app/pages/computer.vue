<script setup lang="ts">
import { useGameArchiveStore } from '../stores/gameArchive'
import { computed, onMounted, ref } from 'vue'
import { useZenSurface } from '../composables/useZenSurface'
import type { PlayerInfo } from '../components/PlayerLine.vue'
import {
  GAME_GRAMMAR,
  spokenCommand,
  spokenMove,
  type GameCommand,
} from '@kchess/rules/voiceCommands'
import { setupPgn } from '@kchess/rules/chess'
import { VARIANT_LABELS } from '@kchess/rules/variant'
import { useAnalysisStore } from '../stores/analysis'
import type { VoiceReply } from '../components/VoiceMoveInput.vue'
import { DEFAULT_ENGINE_LEVELS, engineLevelLabel } from '@kchess/rules/engineLevels'
import { useReplaceAnalysis } from '../composables/useReplaceAnalysis'
import { opposite } from '@kchess/rules/position'

const archive = useGameArchiveStore()
const store = useKChessStore()
useZenSurface(true)
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
  localCanTakeback,
  localCheck,
  localInteractive,
  localCanPlay,
  localResult,
  localGame,
  localSetup,
  settings,
} = storeToRefs(store)
const { newGame, takeback, resign, makeMove, selectPage, openSettings, fen, recheckEngine } = store

// The engine can be removed or replaced while the app is open; look again whenever this page opens.
onMounted(() => void recheckEngine())

/** Only the levels the player keeps in Settings → Gameplay. */
const levels = computed(() =>
  (settings.value?.engineLevels ?? DEFAULT_ENGINE_LEVELS).map((id) => ({
    label: engineLevelLabel(id),
    value: id,
  })),
)
const otherColor = computed(() => opposite(userColor.value))
/** The game's PGN result: the result card says it from the player's side. */
const pgnResult = computed(() => {
  const kind = localResult.value?.kind
  if (!kind) return '*'
  if (kind === 'draw') return '1/2-1/2'
  const winner = kind === 'win' ? userColor.value : otherColor.value
  return winner === 'white' ? '1-0' : '0-1'
})

/** A game with moves that has not finished: leaving it needs a confirmation. */
const inProgress = computed(() => localMoves.value.length > 0 && !localResult.value)
const confirmNew = ref(false)
const confirmResign = ref(false)
const pendingColor = ref<'white' | 'black' | null>(null)
/** A yes/no question is on screen; “confirm” and “cancel” answer it. */
const dialogOpen = computed(() => confirmNew.value || confirmResign.value || !!pendingColor.value)
const voiceEnabled = ref(false)
const voiceContext = computed(
  () => `${localGameEpoch.value}:${userColor.value}:${localMoves.value.join(' ')}`,
)
/** Spoken moves are taken only on your turn… */
const voiceActive = computed(() => localInteractive.value && !thinking.value && !dialogOpen.value)
/** …and the voice bar says why not, otherwise. */
const voiceIdle = computed(() =>
  localResult.value
    ? 'The game is over. Say “new game” to play again.'
    : localPly.value !== localMoves.value.length
      ? 'Go to the last move to keep playing.'
      : 'Wait for Stockfish, then say your move.',
)

const COMMAND_NAMES: Record<GameCommand, string> = {
  takeback: 'take back',
  newGame: 'new game',
  resign: 'resign',
  flip: 'switch sides',
}
/** Game controls by voice, which work whenever the engine is there to play. */
function voiceCommand(text: string): VoiceReply | undefined {
  if (dialogOpen.value) return answerDialog(text)
  const command = spokenCommand(text)
  return command ? { said: COMMAND_NAMES[command], feedback: runCommand(command) } : undefined
}
/** Anything that throws the game away asks first, like its button. */
function runCommand(command: GameCommand): string {
  if (command === 'takeback') {
    if (!localCanTakeback.value) return 'Nothing to take back.'
    takeback()
    return 'Took back your last move.'
  }
  if (command === 'newGame') {
    requestNewGame()
    return confirmNew.value ? 'Start a new game? Say “confirm” or “cancel”.' : 'New game started.'
  }
  if (command === 'resign') {
    if (!inProgress.value) return 'There’s no game in progress to resign.'
    confirmResign.value = true
    return 'Resign this game? Say “confirm” or “cancel”.'
  }
  switchSides()
  return pendingColor.value
    ? 'Switch sides and restart? Say “confirm” or “cancel”.'
    : `You now play ${userColor.value}.`
}
/** Answer the open yes/no question. */
function answerDialog(text: string): VoiceReply {
  const answer = spokenMove(text, localGame.value).kind
  if (answer !== 'confirm' && answer !== 'cancel')
    return {
      said: 'expected confirm or cancel',
      feedback: 'Say “confirm” or “cancel”.',
      understood: false,
    }
  if (answer === 'cancel') {
    confirmNew.value = confirmResign.value = false
    pendingColor.value = null
    return { said: answer, feedback: 'Cancelled.' }
  }
  if (confirmResign.value) {
    confirmResign.value = false
    resign()
    return { said: answer, feedback: 'You resigned. Say “new game” to play again.' }
  }
  if (confirmNew.value) {
    confirmNew.value = false
    newGame()
    return { said: answer, feedback: 'New game started.' }
  }
  confirmColorChange()
  return { said: answer, feedback: `New game: you play ${userColor.value}.` }
}
const analysis = useAnalysisStore()
/** Review the game on the analysis board, opened at the move being looked at. */
const replaceAnalysis = useReplaceAnalysis()
function reviewGame(): void {
  if (!canReview.value) return
  replaceAnalysis(() => {
    if (!analysis.loadPgn(setupPgn(localSetup.value, localMoves.value), localPly.value)) return
    analysis.orientation = userColor.value
    const computer = `Stockfish (${engineLevelLabel(level.value)})`
    analysis.origin = {
      white: userColor.value === 'white' ? 'You' : computer,
      black: userColor.value === 'black' ? 'You' : computer,
      player: userColor.value,
    }
    analysis.startReview()
    selectPage('analysis')
  })
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
  const other = opposite(userColor.value)
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

    <div v-board-fit class="play-layout">
      <PlayBoard
        keep-move-input
        :fen="fen(localDisplay)"
        :orientation="orientation"
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
          <VoiceMoveInput
            v-model:enabled="voiceEnabled"
            source="computer"
            :position="localGame"
            :active="voiceActive"
            :listen="engineReady"
            :idle="voiceIdle"
            :context-key="voiceContext"
            :line="localHistory"
            :grammar="GAME_GRAMMAR"
            :command="voiceCommand"
            hint="Say “E four”, “Knight F three”, or “take back”."
            :play="makeMove"
          />
        </template>
      </PlayBoard>
      <MovePanel
        :moves="localHistory"
        :ply="localPly"
        :fen="localSetup.fen"
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
            <BoardViewToggles />
          </div>
          <OpeningName
            v-if="localSetup.variant === 'standard'"
            :setup="localSetup"
            :moves="localMoves"
            :ply="localPly"
          />
          <div
            class="status-banner steady"
            :class="banner.kind"
            :role="localResult ? 'alert' : 'status'"
          >
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
                  @click="openSettings('engine')"
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
              :disabled="!localCanTakeback"
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
              :result="pgnResult"
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
.level-select {
  min-width: 13rem;
}
</style>
