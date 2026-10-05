<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useLocalStorage } from '@vueuse/core'
import { INITIAL_FEN } from 'chessops/fen'
import {
  VARIANT_HINTS,
  VARIANT_LABELS,
  VARIANTS,
  chess960Fen,
  defaultFen,
  setupStart,
  type GameSetup,
  type Variant,
} from '../../src/shared/variant'
import { formatClock } from '../utils/clock'
import { fen as fenOf, navigatePly, setupPgn } from '../utils/chess'
import { positionProblem } from '../utils/boardEditor'
import { useLocalGameStore, type LocalClock } from '../stores/local'
import { useAnalysisStore } from '../stores/analysis'
import {
  MOVE_GRAMMAR,
  spokenChoice,
  spokenMove,
  type VoiceMoveChoice,
} from '../utils/voiceCommands'
import type { VoiceResult } from '../utils/voiceCapture'
import { heardFields, logVoice, updateVoice } from '../utils/voiceLog'
import type { VoiceOutcome } from '../../src/shared/types'

const store = useKChessStore()
const game = useLocalGameStore()
const analysis = useAnalysisStore()
const { settings, zenActive } = storeToRefs(store)
const route = useRoute()
const mode = useLocalStorage<'board' | 'clock'>('kchess:local-mode', 'board')
const clockLayout = useLocalStorage<'horizontal' | 'vertical'>('kchess:clock-layout', 'horizontal')
const clockRotation = useLocalStorage('kchess:clock-rotation', { top: 0, bottom: 0 })
const clockSides = ['top', 'bottom'] as const
const clockLayoutItems = [
  { label: 'Side by side', value: 'horizontal' },
  { label: 'Stacked', value: 'vertical' },
]
function rotateClock(side: 'top' | 'bottom'): void {
  clockRotation.value = {
    ...clockRotation.value,
    [side]: (clockRotation.value[side] + 90) % 360,
  }
}

/* ── New game form ─────────────────────────────────────────────────── */
const formOpen = ref(false)
const variant = ref<Variant>('standard')
const number960 = ref(Math.floor(Math.random() * 960))
const startFen = ref('')
const timeChoice = ref('none')
const oddsMinutes = ref<number | null>(null)
const CLOCKS = [
  { label: 'No clock', value: 'none' },
  { label: '3+2', value: '3+2' },
  { label: '5+0', value: '5+0' },
  { label: '5+3', value: '5+3' },
  { label: '10+0', value: '10+0' },
  { label: '15+10', value: '15+10' },
  { label: '30+0', value: '30+0' },
  { label: '90+30', value: '90+30' },
]
// The editor and analysis board send a position: start a game from it at once.
onMounted(() => {
  const fen = typeof route.query.fen === 'string' ? route.query.fen : ''
  if (fen && !positionProblem(fen)) {
    mode.value = 'board'
    game.start({ variant: 'standard', fen }, null)
  }
  window.addEventListener('keydown', keydown)
})
onUnmounted(() => window.removeEventListener('keydown', keydown))

const variantItems = VARIANTS.map((key) => ({ label: VARIANT_LABELS[key], value: key }))
const chosenSetup = computed<GameSetup | null>(() => {
  if (variant.value === 'chess960')
    return { variant: 'chess960', fen: chess960Fen(number960.value) }
  if (variant.value === 'standard' && startFen.value.trim()) {
    const fen = startFen.value.trim()
    return positionProblem(fen) ? null : { variant: 'standard', fen }
  }
  return { variant: variant.value, fen: defaultFen(variant.value) }
})
const chosenClock = computed<LocalClock | null>(() => {
  if (timeChoice.value === 'none') return null
  const [minutes = 5, increment = 0] = timeChoice.value.split('+').map(Number)
  return {
    white: { minutes, increment },
    black: { minutes: oddsMinutes.value ?? minutes, increment },
  }
})
function begin(): void {
  if (!chosenSetup.value) return
  game.start(chosenSetup.value, chosenClock.value)
  formOpen.value = false
}

/* ── Board ─────────────────────────────────────────────────────────── */
const players = computed(() => {
  const name = (color: 'white' | 'black') => (color === 'white' ? 'White' : 'Black')
  const top = game.orientation === 'white' ? 'black' : 'white'
  const bottom = game.orientation
  return {
    top: { name: name(top), icon: 'i-lucide-user', clock: game.clockText(top) },
    bottom: { name: name(bottom), icon: 'i-lucide-user', clock: game.clockText(bottom) },
  }
})
const banner = computed(() => {
  const result = game.result
  if (result)
    return {
      icon: result.winner ? 'i-lucide-trophy' : 'i-lucide-handshake',
      title: result.winner ? `${result.winner === 'white' ? 'White' : 'Black'} wins` : 'Draw',
      detail: result.reason,
    }
  if (!game.atLive)
    return {
      icon: 'i-lucide-history',
      title: 'Reviewing an earlier position',
      detail: 'Go to the last move to keep playing.',
    }
  return {
    icon: 'i-lucide-mouse-pointer-click',
    title: `${game.position.turn === 'white' ? 'White' : 'Black'} to move`,
    detail:
      game.times && game.paused && game.moves.length ? 'Clock paused' : game.check ? 'Check!' : '',
  }
})
const variantLabel = computed(() =>
  game.setup.variant === 'standard'
    ? game.setup.fen === INITIAL_FEN
      ? ''
      : 'From position'
    : VARIANT_LABELS[game.setup.variant],
)
const resetKey = ref(0)
function play(uci: string): void {
  if (!game.move(uci)) resetKey.value++
}
const confirmResign = ref<'white' | 'black' | null>(null)
const confirmOpen = computed({
  get: () => confirmResign.value !== null,
  set: (value: boolean) => {
    if (!value) confirmResign.value = null
  },
})

/* Both players can speak moves; every move goes through the normal clock and legality checks. */
const voiceEnabled = ref(false)
const voiceFeedback = ref('')
const voiceChoices = ref<VoiceMoveChoice[]>([])
const voiceActive = computed(
  () =>
    mode.value === 'board' && !game.over && game.atLive && !formOpen.value && !confirmOpen.value,
)
const voiceRevision = ref(0)
const voiceContext = computed(() => `${voiceRevision.value}:${fenOf(game.position)}`)
let choiceContext = ''
let pendingAttempt: Promise<number | undefined> | undefined
function settleVoice(outcome: VoiceOutcome, expected?: string): void {
  const attempt = pendingAttempt
  pendingAttempt = undefined
  if (attempt) void attempt.then((id) => updateVoice(id, { outcome, expected }))
}
watch(
  [() => game.setup, () => game.moves, () => game.shownPly, voiceActive, voiceEnabled],
  () => {
    settleVoice('abandoned')
    voiceChoices.value = []
    voiceFeedback.value = ''
    voiceRevision.value++
  },
  { flush: 'sync' },
)
function recordVoice(result: VoiceResult, outcome: VoiceOutcome, parsed?: string) {
  return logVoice({
    source: 'local',
    ...heardFields(result),
    outcome,
    parsed,
    fen: fenOf(game.position),
  })
}
function playVoice(choice: VoiceMoveChoice, outcome: 'played' | 'confirmed' | 'picked'): void {
  if (!voiceEnabled.value || !voiceActive.value || choiceContext !== voiceContext.value) return
  // Settle before moving: the position watcher invalidates any remaining pending choices.
  settleVoice(outcome, choice.san)
  voiceChoices.value = []
  voiceFeedback.value = game.move(choice.uci)
    ? `Played ${choice.san}.`
    : 'That move is no longer legal.'
}
function cancelVoice(): void {
  settleVoice('cancelled')
  voiceChoices.value = []
  voiceFeedback.value = 'Move cancelled.'
}
function hearMove(result: VoiceResult): void {
  if (!voiceEnabled.value || !voiceActive.value) return
  const index = spokenChoice(result.text)
  if (index && voiceChoices.value.length && !result.unclear) {
    void recordVoice(result, 'command', `#${index}`)
    const choice = voiceChoices.value[index - 1]
    if (choice) playVoice(choice, 'confirmed')
    else voiceFeedback.value = 'Choose one of the numbered moves.'
    return
  }
  const parsed = spokenMove(result.text, game.position)
  if (parsed.kind === 'cancel' && !result.unclear) {
    void recordVoice(result, 'command', 'cancel')
    cancelVoice()
  } else if (parsed.kind === 'confirm' && !result.unclear) {
    void recordVoice(result, 'command', 'confirm')
    if (voiceChoices.value.length === 1) playVoice(voiceChoices.value[0]!, 'confirmed')
    else voiceFeedback.value = 'Say a move, or choose its number.'
  } else if (parsed.kind === 'move') {
    settleVoice('replaced')
    choiceContext = voiceContext.value
    voiceChoices.value = parsed.choices
    pendingAttempt = recordVoice(
      result,
      'pending',
      parsed.choices.map((choice) => choice.san).join('|'),
    )
    if (parsed.choices.length === 1 && !settings.value.voiceConfirmMoves && !result.unclear)
      playVoice(parsed.choices[0]!, 'played')
    else
      voiceFeedback.value =
        parsed.choices.length === 1
          ? `Play ${parsed.choices[0]!.san}? Say “confirm” or “cancel”.`
          : 'Several moves match: say its number, or select it.'
  } else {
    void recordVoice(result, result.unclear ? 'unclear' : 'invalid')
    voiceFeedback.value = result.unclear
      ? 'Didn’t catch that. Repeat, or try “Echo four”.'
      : `Heard “${result.text}”: no legal move matches.`
  }
}
function missedMove(result: VoiceResult): void {
  if (voiceEnabled.value && voiceActive.value) void recordVoice(result, 'unclear')
}
onUnmounted(() => settleVoice('abandoned'))
function analyse(): void {
  if (game.setup.variant !== 'standard') return
  if (!analysis.loadPgn(setupPgn(game.setup, game.moves), game.shownPly)) return
  analysis.origin = { white: 'White', black: 'Black' }
  store.selectPage('analysis')
}
const toast = useToast()
async function copyPgn(): Promise<void> {
  const result = game.result
  const headers: Record<string, string> = {
    Event: 'Over the board (KChess)',
    Date: new Date().toISOString().slice(0, 10).replace(/-/g, '.'),
    White: 'White',
    Black: 'Black',
    Result: result
      ? result.winner === 'white'
        ? '1-0'
        : result.winner === 'black'
          ? '0-1'
          : '1/2-1/2'
      : '*',
  }
  try {
    await navigator.clipboard.writeText(setupPgn(game.setup, game.moves, headers))
    toast.add({ title: 'PGN copied', icon: 'i-lucide-clipboard-check' })
  } catch {
    toast.add({ title: 'Could not copy the PGN', color: 'error' })
  }
}
function keydown(event: KeyboardEvent): void {
  if (
    event.target instanceof HTMLElement &&
    (event.target.closest('input, textarea, select, [contenteditable="true"]') ||
      event.target.closest('[role="dialog"], [role="listbox"]'))
  )
    return
  if (mode.value === 'clock') {
    // Space presses the running clock, like the button on a real clock; P pauses.
    if (event.key === ' ' && event.target === document.body) {
      event.preventDefault()
      game.otbPress(game.otbRunning ?? 'top')
    } else if (event.key === 'p') game.otbPause()
    return
  }
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement)
    return
  const next = navigatePly(event.key, game.shownPly, game.moves.length)
  if (next !== undefined) {
    event.preventDefault()
    game.view(next)
  }
}
const setupProblem = computed(() =>
  variant.value === 'standard' && startFen.value.trim()
    ? (positionProblem(startFen.value.trim()) ??
      (setupStart({ variant: 'standard', fen: startFen.value.trim() })
        ? ''
        : 'Not a legal position.'))
    : '',
)

/* ── Chess clock ──────────────────────────────────────────────────── */
function otbText(side: 'top' | 'bottom'): string {
  const ms = game.otbLeft(side)
  return ms < 10_000
    ? `0:${(ms / 1000).toFixed(1).padStart(4, '0')}`
    : formatClock(Math.ceil(ms / 1000) * 1000)
}
</script>

<template>
  <div class="local-page">
    <PageHeader title="Over the board">
      <UTabs
        v-model="mode"
        :items="[
          { label: 'Shared board', value: 'board', icon: 'i-lucide-users-round' },
          { label: 'Chess clock', value: 'clock', icon: 'i-lucide-timer' },
        ]"
        :content="false"
        size="sm"
        variant="pill"
      />
    </PageHeader>
    <p v-if="game.saveError" role="alert" class="p-3 text-error">
      Automatic saving: {{ game.saveError }}
    </p>

    <div v-if="mode === 'board'" class="play-layout">
      <PlayBoard
        :fen="fenOf(game.display)"
        :orientation="game.orientation"
        :theme="settings.boardTheme"
        :coordinates="settings.coordinates"
        :piece-set="settings.pieceSet"
        :animation="settings.pieceAnimation"
        :movable="!game.over && game.atLive"
        :interactive="!game.over && game.atLive"
        :movable-color="game.position.turn"
        :show-dests="settings.showLegalMoves"
        :promotion="settings.promotion === 'queen' ? 'queen' : 'ask'"
        :dests="game.dests"
        :last-move="game.lastMove"
        :check="game.check"
        :turn-color="game.display.turn"
        :live="!game.over"
        :top="players.top"
        :bottom="players.bottom"
        :blindfold="settings.blindfold && !game.over"
        :variant="game.setup.variant"
        :reset-key="resetKey"
        @move="play"
      >
        <template #bottom-aside>
          <VoiceInput
            v-model:enabled="voiceEnabled"
            compact
            :active="voiceActive"
            :context-key="voiceContext"
            :grammar="MOVE_GRAMMAR"
            hint="Either player can say “E four” or “Knight F three”."
            :feedback="voiceFeedback"
            allow-push-talk
            accept-unclear
            @result="hearMove"
            @unclear="missedMove"
          >
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
            <UButton
              v-if="voiceChoices.length"
              color="neutral"
              variant="ghost"
              size="xs"
              icon="i-lucide-x"
              aria-label="Cancel move"
              @click="cancelVoice"
            />
          </VoiceInput>
        </template>
      </PlayBoard>
      <MovePanel
        title="Moves"
        :moves="game.history"
        :ply="game.shownPly"
        empty-text="White moves first. The first move starts the clock."
        @select="game.view($event)"
        @flip="game.flip()"
      >
        <template #top>
          <div class="flex flex-wrap items-center gap-2 text-xs muted">
            <span v-if="variantLabel" class="font-semibold">{{ variantLabel }}</span>
            <span v-if="game.clock"
              >{{ game.clock.white.minutes }}+{{ game.clock.white.increment
              }}<template v-if="game.clock.black.minutes !== game.clock.white.minutes">
                (Black {{ game.clock.black.minutes }} min)</template
              ></span
            >
            <span class="flex-1" />
            <USwitch v-model="game.autoFlip" size="xs" label="Turn the board" />
            <UTooltip text="Blindfold: hide the pieces">
              <UButton
                size="xs"
                :variant="settings.blindfold ? 'soft' : 'ghost'"
                :color="settings.blindfold ? 'primary' : 'neutral'"
                icon="i-lucide-eye-off"
                aria-label="Blindfold"
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
                @click="store.toggleSetting('zenMode')"
              />
            </UTooltip>
          </div>
          <OpeningName
            v-if="game.setup.variant === 'standard'"
            :setup="game.setup"
            :moves="game.moves"
            :ply="game.shownPly"
          />
          <div class="status-banner" :class="{ win: game.result?.winner }" role="status">
            <UIcon :name="banner.icon" class="status-icon" />
            <div>
              <strong>{{ banner.title }}</strong>
              <span v-if="banner.detail" class="detail">{{ banner.detail }}</span>
            </div>
          </div>
        </template>
        <template #bottom>
          <div class="panel-actions panel-divider pt-3.5">
            <UButton
              variant="outline"
              color="neutral"
              icon="i-lucide-undo-2"
              :disabled="!game.moves.length"
              @click="game.takeback()"
              >Take back</UButton
            >
            <UButton
              v-if="game.times && !game.over"
              variant="outline"
              color="neutral"
              :icon="game.paused ? 'i-lucide-play' : 'i-lucide-pause'"
              :disabled="!game.moves.length"
              @click="game.togglePause()"
              >{{ game.paused ? 'Resume' : 'Pause' }}</UButton
            >
            <template v-if="!game.over && game.moves.length">
              <UButton
                variant="outline"
                color="neutral"
                icon="i-lucide-handshake"
                @click="game.agreeDraw()"
                >Draw</UButton
              >
              <UButton
                variant="outline"
                color="error"
                icon="i-lucide-flag"
                @click="confirmResign = game.position.turn"
                >Resign</UButton
              >
            </template>
            <UButton icon="i-lucide-plus" @click="formOpen = true">New game</UButton>
          </div>
          <div class="panel-actions zen-hide">
            <UButton
              size="xs"
              variant="ghost"
              color="neutral"
              icon="i-lucide-copy"
              :disabled="!game.moves.length"
              @click="copyPgn"
              >Copy PGN</UButton
            >
            <ExportGame
              :setup="game.setup"
              :moves="game.moves"
              :orientation="game.orientation"
              white="White"
              black="Black"
            />
            <UButton
              v-if="game.setup.variant === 'standard'"
              size="xs"
              variant="ghost"
              color="neutral"
              icon="i-lucide-microscope"
              :disabled="!game.moves.length"
              @click="analyse"
              >Analyse</UButton
            >
          </div>
        </template>
      </MovePanel>
    </div>

    <div v-else class="otb-clock">
      <div class="otb-display-controls">
        <label class="otb-layout-picker">
          <span class="muted text-sm">Layout</span>
          <USelect
            v-model="clockLayout"
            :items="clockLayoutItems"
            aria-label="Clock layout"
            class="w-36"
            size="sm"
          />
        </label>
        <div class="otb-rotation-controls">
          <UButton
            v-for="(side, index) in clockSides"
            :key="side"
            size="sm"
            variant="outline"
            color="neutral"
            icon="i-lucide-rotate-cw"
            :aria-label="`Rotate Player ${index + 1} clock, currently ${clockRotation[side]} degrees`"
            @click="rotateClock(side)"
            >Player {{ index + 1 }} · {{ clockRotation[side] }}°</UButton
          >
        </div>
      </div>
      <div class="otb-faces" :class="{ stacked: clockLayout === 'vertical' }">
        <button
          v-for="(side, index) in clockSides"
          :key="side"
          type="button"
          class="otb-side"
          :class="{
            active: game.otbRunning === side,
            flagged: game.otbFlagged === side,
            sideways: clockRotation[side] === 90 || clockRotation[side] === 270,
          }"
          :aria-label="`Player ${index + 1} clock, ${otbText(side)}. Press after your move.`"
          @click="game.otbPress(side)"
        >
          <span class="otb-face" :style="{ transform: `rotate(${clockRotation[side]}deg)` }">
            <span class="otb-player">Player {{ index + 1 }}</span>
            <span class="otb-time tabular">{{ otbText(side) }}</span>
            <span class="otb-moves">{{ game.otbMoves[side] }} moves</span>
            <span class="otb-state">
              {{
                game.otbFlagged === side
                  ? 'Time up'
                  : game.otbRunning === side
                    ? 'Your turn'
                    : game.otbRunning
                      ? 'Waiting'
                      : 'Ready'
              }}
            </span>
          </span>
        </button>
      </div>
      <div class="otb-controls">
        <UButton
          size="sm"
          variant="outline"
          color="neutral"
          :icon="game.otbRunning ? 'i-lucide-pause' : 'i-lucide-play'"
          @click="game.otbRunning ? game.otbPause() : game.otbPress('top')"
          >{{ game.otbRunning ? 'Pause' : 'Start' }}</UButton
        >
        <UButton
          size="sm"
          variant="outline"
          color="neutral"
          icon="i-lucide-rotate-ccw"
          @click="game.otbReset()"
          >Reset</UButton
        >
        <label class="flex items-center gap-1 text-sm">
          <UInput
            v-model.number="game.otbConfig.minutes"
            type="number"
            :min="1"
            :max="180"
            size="xs"
            class="w-16"
            aria-label="Player 1 minutes"
          />
          <span class="muted">/</span>
          <UInput
            v-model.number="game.otbConfig.bottomMinutes"
            type="number"
            :min="1"
            :max="180"
            size="xs"
            class="w-16"
            aria-label="Player 2 minutes"
          />
          min +
          <UInput
            v-model.number="game.otbConfig.increment"
            type="number"
            :min="0"
            :max="180"
            size="xs"
            class="w-16"
            aria-label="Increment seconds"
          />
          s
        </label>
        <span class="otb-shortcuts muted text-xs">Space switches turns · P pauses</span>
      </div>
    </div>

    <UModal v-model:open="formOpen" title="New game over the board">
      <template #body>
        <form class="flex flex-col gap-4" @submit.prevent="begin">
          <UFormField label="Variant" :help="VARIANT_HINTS[variant]">
            <USelect v-model="variant" :items="variantItems" class="w-full" />
          </UFormField>
          <div v-if="variant === 'chess960'" class="flex items-end gap-2">
            <UFormField label="Position number (0–959)" class="flex-1">
              <UInput v-model.number="number960" type="number" :min="0" :max="959" />
            </UFormField>
            <UButton
              variant="outline"
              color="neutral"
              icon="i-lucide-shuffle"
              @click="number960 = Math.floor(Math.random() * 960)"
              >Random</UButton
            >
          </div>
          <UFormField
            v-if="variant === 'standard'"
            label="Start from a FEN (optional)"
            :error="setupProblem || undefined"
          >
            <UInput v-model="startFen" class="w-full" placeholder="Standard start" />
          </UFormField>
          <div class="grid grid-cols-2 gap-3">
            <UFormField label="Clock">
              <USelect v-model="timeChoice" :items="CLOCKS" class="w-full" />
            </UFormField>
            <UFormField label="Black's minutes (odds)" help="Leave empty for equal time">
              <UInput
                v-model.number="oddsMinutes"
                type="number"
                :min="1"
                :max="180"
                :disabled="timeChoice === 'none'"
              />
            </UFormField>
          </div>
          <div class="flex justify-end gap-2">
            <UButton color="neutral" variant="ghost" @click="formOpen = false">Cancel</UButton>
            <UButton type="submit" :disabled="!chosenSetup">Start</UButton>
          </div>
        </form>
      </template>
    </UModal>
    <ConfirmDialog
      v-model:open="confirmOpen"
      :title="`Resign for ${confirmResign === 'white' ? 'White' : 'Black'}?`"
      description="The other side wins. The game stays on the board."
      confirm-label="Resign"
      color="error"
      @confirm="confirmResign && game.resign(confirmResign)"
    />
  </div>
</template>

<style scoped>
.local-page :deep(.page-header) {
  max-width: calc(var(--board-size) + 24px + 440px);
  margin-inline: auto;
}
.otb-clock {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-height: 460px;
  height: calc(100dvh - 190px);
}
.otb-display-controls,
.otb-layout-picker,
.otb-rotation-controls {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}
.otb-display-controls {
  justify-content: space-between;
}
.otb-faces {
  flex: 1;
  min-height: 300px;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
}
.otb-faces.stacked {
  grid-template-columns: minmax(0, 1fr);
  grid-template-rows: repeat(2, minmax(0, 1fr));
  min-height: 480px;
}
.otb-side {
  container-type: size;
  min-width: 0;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  border-radius: 18px;
  border: 1px solid var(--ui-border);
  background: var(--ui-bg-elevated);
  transition: background 0.15s;
  cursor: pointer;
  touch-action: manipulation;
  user-select: none;
}
.otb-side:focus-visible {
  outline: 3px solid var(--ui-primary);
  outline-offset: 3px;
}
.otb-face {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  pointer-events: none;
}
.otb-side.active {
  background: var(--ui-primary);
  color: var(--ui-bg);
}
.otb-side.flagged {
  background: var(--ui-error);
  color: var(--ui-bg);
}
.otb-player {
  font-size: 14px;
  font-weight: 600;
  opacity: 0.75;
}
.otb-time {
  font-size: clamp(32px, min(22cqw, 36cqh), 140px);
  font-weight: 700;
  line-height: 1;
  letter-spacing: -0.04em;
}
.otb-side.sideways .otb-time {
  font-size: clamp(32px, 20cqmin, 140px);
}
.otb-moves {
  font-size: 14px;
  opacity: 0.75;
}
.otb-state {
  font-size: 12px;
  font-weight: 600;
}
.otb-controls {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: center;
  gap: 8px;
}
.otb-shortcuts {
  flex-basis: 100%;
  text-align: center;
}
@media (max-width: 640px) {
  .otb-clock {
    height: auto;
    min-height: calc(100dvh - 190px);
  }
  .otb-faces {
    min-height: 320px;
  }
  .otb-faces.stacked {
    min-height: 480px;
  }
}
</style>
