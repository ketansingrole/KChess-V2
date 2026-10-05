<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { INITIAL_FEN } from 'chessops/fen'
import { DEFAULT_ENGINE_LEVELS, engineLevelLabel } from '../../src/shared/engineLevels'
import { chess960Fen, setupStart, type GameSetup } from '../../src/shared/variant'
import { positionProblem } from '../utils/boardEditor'

const open = defineModel<boolean>('open', { default: false })
const store = useKChessStore()
const { level, userColor, localSetup, localClock, settings } = storeToRefs(store)

type Start = 'standard' | 'chess960' | 'fen'
const start = ref<Start>('standard')
const number960 = ref(518)
const fen = ref('')
const clock = ref('none')
const color = ref<'white' | 'black'>('white')
const chosenLevel = ref(level.value)

const CLOCKS = [
  { label: 'No clock', value: 'none' },
  { label: '3+2', value: '3+2' },
  { label: '5+3', value: '5+3' },
  { label: '10+5', value: '10+5' },
  { label: '15+10', value: '15+10' },
  { label: '30+0', value: '30+0' },
  { label: '60+30', value: '60+30' },
]
const levels = computed(() =>
  (settings.value?.engineLevels ?? DEFAULT_ENGINE_LEVELS).map((id) => ({
    label: engineLevelLabel(id),
    value: id,
  })),
)
// Open on the current game's options.
watch(open, (isOpen) => {
  if (!isOpen) return
  const setup = localSetup.value
  start.value =
    setup.variant === 'chess960' ? 'chess960' : setup.fen === INITIAL_FEN ? 'standard' : 'fen'
  fen.value = start.value === 'fen' ? setup.fen : ''
  clock.value = localClock.value
    ? `${localClock.value.minutes}+${localClock.value.increment}`
    : 'none'
  color.value = userColor.value
  chosenLevel.value = level.value
})
const fenProblem = computed(() => {
  if (start.value !== 'fen') return ''
  const text = fen.value.trim()
  if (!text) return 'Paste a FEN, or set up the position in the board editor.'
  return (
    positionProblem(text) ??
    (setupStart({ variant: 'standard', fen: text }) ? '' : 'That FEN is not a legal position.')
  )
})
const chosenSetup = computed<GameSetup>(() =>
  start.value === 'chess960'
    ? { variant: 'chess960', fen: chess960Fen(number960.value) }
    : start.value === 'fen'
      ? { variant: 'standard', fen: fen.value.trim() }
      : { variant: 'standard', fen: INITIAL_FEN },
)
function randomize(): void {
  number960.value = Math.floor(Math.random() * 960)
}
function begin(): void {
  if (fenProblem.value) return
  const [minutes, increment] = clock.value === 'none' ? [] : clock.value.split('+').map(Number)
  level.value = chosenLevel.value
  userColor.value = color.value
  store.newGame(
    chosenSetup.value,
    minutes === undefined ? null : { minutes, increment: increment ?? 0 },
  )
  open.value = false
}
</script>

<template>
  <UModal v-model:open="open" title="New game against Stockfish">
    <template #body>
      <form class="flex flex-col gap-4" @submit.prevent="begin">
        <UFormField label="Start from">
          <UTabs
            v-model="start"
            :items="[
              { label: 'Standard', value: 'standard' },
              { label: 'Chess960', value: 'chess960' },
              { label: 'A position', value: 'fen' },
            ]"
            :content="false"
            size="sm"
            variant="pill"
            class="w-full"
          />
        </UFormField>
        <div v-if="start === 'chess960'" class="flex items-end gap-2">
          <UFormField label="Position number (0–959)" class="flex-1">
            <UInput v-model.number="number960" type="number" :min="0" :max="959" />
          </UFormField>
          <UButton variant="outline" color="neutral" icon="i-lucide-shuffle" @click="randomize"
            >Random</UButton
          >
        </div>
        <div v-if="start === 'chess960'" class="mini-preview">
          <ChessBoard
            :fen="chosenSetup.fen"
            orientation="white"
            :theme="settings.boardTheme"
            coordinates="none"
            :piece-set="settings.pieceSet"
            animation="none"
            :interactive="false"
          />
        </div>
        <UFormField v-if="start === 'fen'" label="FEN" :error="fenProblem || undefined">
          <UTextarea
            v-model="fen"
            :rows="2"
            class="w-full"
            placeholder="rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
          />
        </UFormField>
        <div class="grid grid-cols-2 gap-3">
          <UFormField label="Clock">
            <USelect v-model="clock" :items="CLOCKS" class="w-full" />
          </UFormField>
          <UFormField label="You play">
            <USelect
              v-model="color"
              :items="[
                { label: 'White', value: 'white' },
                { label: 'Black', value: 'black' },
              ]"
              class="w-full"
            />
          </UFormField>
        </div>
        <UFormField label="Stockfish strength">
          <USelect v-model="chosenLevel" :items="levels" class="w-full" />
        </UFormField>
        <div class="flex justify-end gap-2">
          <UButton color="neutral" variant="ghost" @click="open = false">Cancel</UButton>
          <UButton type="submit" :disabled="Boolean(fenProblem)">Start game</UButton>
        </div>
      </form>
    </template>
  </UModal>
</template>

<style scoped>
.mini-preview {
  width: 180px;
  align-self: center;
}
</style>
