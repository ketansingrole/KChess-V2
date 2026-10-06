<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import {
  ARENA_CLOCK_MINUTES,
  ARENA_DURATIONS,
  ARENA_INCREMENTS,
  ARENA_WAIT_MINUTES,
  type NewArena,
} from '../../src/shared/types'
import { canBoardSeek } from '../../src/shared/timeControl'
import { useTournamentStore } from '../stores/tournaments'

/** Creates a Lichess arena run by the active account. */
const open = defineModel<boolean>('open', { default: false })
const store = useKChessStore()
const tournaments = useTournamentStore()

const VARIANTS = [
  { label: 'Standard', value: 'standard' },
  { label: 'Chess960', value: 'chess960' },
  { label: 'King of the Hill', value: 'kingOfTheHill' },
  { label: 'Three-check', value: 'threeCheck' },
  { label: 'Antichess', value: 'antichess' },
  { label: 'Atomic', value: 'atomic' },
  { label: 'Horde', value: 'horde' },
  { label: 'Racing Kings', value: 'racingKings' },
  { label: 'Crazyhouse', value: 'crazyhouse' },
]
const fraction: Record<number, string> = { 0.25: '¼', 0.5: '½', 0.75: '¾' }
const clockItems = ARENA_CLOCK_MINUTES.filter((m) => m > 0).map((m) => ({
  label: `${fraction[m] ?? m} min`,
  value: m,
}))
const incrementItems = ARENA_INCREMENTS.map((s) => ({ label: `+${s} s`, value: s }))
const durationItems = ARENA_DURATIONS.map((m) => ({
  label: m < 60 || m % 60 ? `${m} min` : `${m / 60} h`,
  value: m,
}))
const startItems = [
  ...ARENA_WAIT_MINUTES.map((m) => ({ label: `In ${m} min`, value: String(m) })),
  { label: 'At a set time', value: 'later' },
]

const name = ref('')
const clockTime = ref<NewArena['clockTime']>(3)
const clockIncrement = ref<NewArena['clockIncrement']>(2)
const minutes = ref<NewArena['minutes']>(60)
const start = ref('5')
const startAt = ref('')
const variant = ref('standard')
const rated = ref(true)
const password = ref('')
const description = ref('')
const busy = ref(false)
const error = ref('')
const reconnect = ref(false)

watch(open, (now) => {
  if (!now) return
  error.value = ''
  reconnect.value = false
  // Default a set start time to the next round hour.
  const next = new Date(Date.now() + 60 * 60_000)
  next.setMinutes(0, 0, 0)
  const pad = (n: number) => String(n).padStart(2, '0')
  startAt.value = `${next.getFullYear()}-${pad(next.getMonth() + 1)}-${pad(next.getDate())}T${pad(next.getHours())}:00`
})

/** Lichess lets third-party apps play only Rapid and Classical arena games. */
const playableHere = computed(
  () => variant.value !== 'crazyhouse' && canBoardSeek(clockTime.value, clockIncrement.value),
)
const startDate = computed(() =>
  start.value === 'later' && startAt.value ? new Date(startAt.value).getTime() : undefined,
)
const startProblem = computed(() =>
  start.value === 'later' && (!startDate.value || startDate.value < Date.now() + 60_000)
    ? 'Pick a start time in the future.'
    : '',
)

async function submit(): Promise<void> {
  if (busy.value || startProblem.value) return
  busy.value = true
  error.value = ''
  reconnect.value = false
  const result = await tournaments.create({
    name: name.value.trim() || undefined,
    clockTime: clockTime.value,
    clockIncrement: clockIncrement.value,
    minutes: minutes.value,
    waitMinutes:
      start.value === 'later' ? undefined : (Number(start.value) as NewArena['waitMinutes']),
    startDate: startDate.value,
    variant: variant.value,
    rated: rated.value,
    password: password.value || undefined,
    description: description.value.trim() || undefined,
  })
  busy.value = false
  if (result.reconnect) reconnect.value = true
  else if (result.error) error.value = result.error
  else {
    open.value = false
    name.value = ''
    password.value = ''
    description.value = ''
    store.notifyInfo(`Created ${result.created!.name}.`)
    void navigateTo({ path: '/tournaments', query: { system: 'arena', id: result.created!.id } })
  }
}
</script>

<template>
  <UModal v-model:open="open" title="Create an arena" :ui="{ content: 'max-w-xl' }">
    <template #body>
      <form id="create-arena" class="arena-form" @submit.prevent="submit">
        <UFormField label="Name" class="span-2">
          <UInput
            v-model="name"
            :maxlength="30"
            placeholder="Leave empty for a random name"
            class="w-full"
          />
        </UFormField>
        <UFormField label="Clock">
          <USelect v-model="clockTime" :items="clockItems" class="w-full" />
        </UFormField>
        <UFormField label="Increment">
          <USelect v-model="clockIncrement" :items="incrementItems" class="w-full" />
        </UFormField>
        <UFormField label="Duration">
          <USelect v-model="minutes" :items="durationItems" class="w-full" />
        </UFormField>
        <UFormField label="Variant">
          <USelect v-model="variant" :items="VARIANTS" class="w-full" />
        </UFormField>
        <UFormField label="Starts" :class="{ 'span-2': start !== 'later' }">
          <USelect v-model="start" :items="startItems" class="w-full" />
        </UFormField>
        <UFormField v-if="start === 'later'" label="Start time" :error="startProblem || undefined">
          <UInput v-model="startAt" type="datetime-local" class="w-full" />
        </UFormField>
        <UFormField label="Entry code" hint="Optional" class="span-2">
          <UInput
            v-model="password"
            type="password"
            autocomplete="off"
            placeholder="Players need it to join"
            class="w-full"
          />
        </UFormField>
        <UFormField label="Description" hint="Optional" class="span-2">
          <UTextarea v-model="description" :rows="2" :maxlength="2000" autoresize class="w-full" />
        </UFormField>
        <div class="span-2 flex items-center justify-between gap-3">
          <span id="arena-rated-label" class="text-sm">Rated</span>
          <USwitch v-model="rated" aria-labelledby="arena-rated-label" />
        </div>
      </form>
      <UAlert
        v-if="!playableHere"
        class="mt-4"
        color="neutral"
        variant="subtle"
        icon="i-lucide-eye"
        description="KChess can play only Rapid and Classical arena games; players will join faster ones on Lichess."
      />
      <UAlert
        v-if="reconnect"
        class="mt-4"
        color="warning"
        variant="subtle"
        icon="i-lucide-key-round"
        title="Lichess needs a new permission"
        :description="`Connect @${store.activeOnlineAccount} again to allow creating tournaments.`"
        :actions="[{ label: 'Reconnect', icon: 'i-lucide-link', onClick: () => store.connect() }]"
      />
      <p v-if="error" class="text-error text-sm mt-3" role="alert">{{ error }}</p>
    </template>
    <template #footer>
      <div class="flex w-full justify-end gap-2">
        <UButton variant="ghost" color="neutral" @click="open = false">Cancel</UButton>
        <UButton
          type="submit"
          form="create-arena"
          icon="i-lucide-plus"
          :loading="busy"
          :disabled="Boolean(startProblem)"
          >Create arena</UButton
        >
      </div>
    </template>
  </UModal>
</template>

<style scoped>
.arena-form {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px 16px;
}
.span-2 {
  grid-column: 1 / -1;
}
@media (max-width: 520px) {
  .arena-form {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
