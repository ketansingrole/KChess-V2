<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import type { PlayerInfo } from '../components/PlayerLine.vue'

const store = useKChessStore()
const {
  localMoves,
  localPly,
  level,
  userColor,
  flipped,
  thinking,
  engineReady,
  engineChecking,
  engineName,
  localDisplay,
  localHistory,
  localLast,
  localDests,
  localTurn,
  localCheck,
  localInteractive,
  localCanPlay,
  localResult,
  settings,
} = storeToRefs(store)
const { newGame, takeback, makeMove, selectPage, fen, recheckEngine } = store

// The engine can be removed or replaced while the app is open; look again whenever this page opens.
onMounted(() => void recheckEngine())

const levels = [
  { label: 'Low · 1350', value: 'low' },
  { label: 'Medium · 1800', value: 'medium' },
  { label: 'High · Max', value: 'high' },
] as const
const levelName = computed(() => level.value[0]!.toUpperCase() + level.value.slice(1))

const colors = [
  { label: 'White', value: 'white', icon: 'i-lucide-circle' },
  { label: 'Black', value: 'black', icon: 'i-lucide-circle-dot' },
] as const

/** A game with moves that has not finished: leaving it needs a confirmation. */
const inProgress = computed(() => localMoves.value.length > 0 && !localResult.value)
const confirmNew = ref(false)
const pendingColor = ref<'white' | 'black' | null>(null)
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
function requestColor(choice: string | number): void {
  const value = choice === 'black' ? 'black' : 'white'
  if (value === userColor.value) return
  if (inProgress.value) {
    pendingColor.value = value
    return
  }
  applyColor(value)
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

const orientation = computed(() => (flipped.value ? 'black' : 'white'))
/** Labels follow the board orientation, so flipping the board never mislabels a side. */
const players = computed<{ top: PlayerInfo; bottom: PlayerInfo }>(() => {
  const computerPlayer: PlayerInfo = {
    name: `Stockfish · ${levelName.value}`,
    icon: 'i-lucide-cpu',
    status: thinking.value ? 'Thinking…' : undefined,
    busy: thinking.value,
    presence: engineChecking.value
      ? { state: 'checking', label: 'Checking…' }
      : engineReady.value
        ? { state: 'online', label: 'Live' }
        : { state: 'unavailable', label: 'Unavailable' },
  }
  const you: PlayerInfo = { name: 'You', icon: 'i-lucide-user' }
  const youAtBottom = orientation.value === userColor.value
  return youAtBottom ? { top: computerPlayer, bottom: you } : { top: you, bottom: computerPlayer }
})
</script>

<template>
  <div>
    <PageHeader title="Play with Computer" subtitle="Challenge Stockfish at your level">
      <UBadge
        v-if="engineChecking && !engineReady"
        color="neutral"
        variant="soft"
        icon="i-lucide-loader-circle"
        size="lg"
        :ui="{ leadingIcon: 'animate-spin' }"
        role="status"
        >Checking Stockfish…</UBadge
      >
      <UBadge
        v-else-if="engineReady"
        color="success"
        variant="soft"
        icon="i-lucide-circle-check"
        size="lg"
        role="status"
        >Stockfish live · {{ engineName }}</UBadge
      >
      <template v-else>
        <UBadge color="error" variant="soft" icon="i-lucide-circle-x" size="lg" role="status"
          >Stockfish unavailable</UBadge
        >
        <UButton
          color="neutral"
          variant="outline"
          icon="i-lucide-rotate-cw"
          :loading="engineChecking"
          @click="recheckEngine"
          >Check again</UButton
        >
        <UButton
          color="warning"
          variant="soft"
          icon="i-lucide-settings-2"
          @click="selectPage('settings')"
          >Set up Stockfish</UButton
        >
      </template>
    </PageHeader>

    <div class="play-layout">
      <PlayBoard
        :fen="fen(localDisplay)"
        :orientation="orientation"
        :theme="settings.boardTheme"
        :coordinates="settings.coordinates"
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
        @move="makeMove"
      />
      <MovePanel
        :moves="localHistory"
        :ply="localPly"
        empty-text="Make your move to start the game."
        @select="localPly = $event"
        @flip="flipped = !flipped"
      >
        <template #top>
          <!-- Each control gets a full-width row: side by side they were too narrow for their labels. -->
          <div class="form-stack">
            <div class="field">
              <span id="difficulty-label" class="field-label">Difficulty</span>
              <USelect
                v-model="level"
                :items="[...levels]"
                aria-labelledby="difficulty-label"
                size="sm"
                :ui="{ base: 'w-full' }"
              />
            </div>
            <div class="field">
              <span id="play-as-label" class="field-label">Play as</span>
              <UTabs
                :model-value="userColor"
                :items="[...colors]"
                aria-labelledby="play-as-label"
                size="sm"
                :content="false"
                variant="pill"
                class="w-full"
                :ui="{ trigger: 'grow' }"
                @update:model-value="requestColor"
              />
            </div>
          </div>
          <div class="status-banner" :class="banner.kind" :role="localResult ? 'alert' : 'status'">
            <UIcon
              :name="banner.icon"
              class="status-icon"
              :class="{ 'animate-spin': banner.spin }"
            />
            <div>
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
              <UButton
                v-if="!engineReady"
                size="xs"
                variant="link"
                color="neutral"
                class="px-0"
                @click="selectPage('settings')"
                >Open Settings</UButton
              >
            </div>
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
              :variant="localResult ? 'solid' : 'outline'"
              :color="localResult ? 'primary' : 'neutral'"
              icon="i-lucide-rotate-ccw"
              @click="requestNewGame"
              >New game</UButton
            >
          </div>
        </template>
      </MovePanel>
    </div>

    <ConfirmDialog
      v-model:open="confirmNew"
      title="Start a new game?"
      description="Your current game is still in progress and will be lost."
      confirm-label="New game"
      @confirm="newGame"
    />
    <ConfirmDialog
      v-model:open="confirmColor"
      title="Switch sides?"
      description="Your current game is still in progress. Switching sides starts a new game."
      confirm-label="Switch and restart"
      @confirm="confirmColorChange"
    />
  </div>
</template>
