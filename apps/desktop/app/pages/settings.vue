<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import type {
  EngineLevel,
  LichessAccount,
  MicrophoneStatus,
  NotificationSkip,
} from '@kchess/contracts/types'
import { DEFAULT_ENGINE_LEVELS, ENGINE_LADDER } from '@kchess/rules/engineLevels'
import { LAUNCHER_PERMISSION_HINT, useMicrophoneAccess } from '../utils/microphone'
import { describeMicError } from '../utils/voiceCapture'
import { formatBytes, formatCount } from '../utils/format'
import { pieceSets, pieceVars } from '../utils/pieces'
import { previewColors } from '../utils/themes'
import { SETTINGS_SECTIONS } from '../stores/kchess'

const store = useKChessStore()
const usage = useUsageStore()
const {
  busy,
  data,
  settings,
  connectedAccounts,
  activeOnlineAccount,
  settingsSection,
  themeList,
  themeProblems,
  themesDir,
} = storeToRefs(store)
const {
  boardThemes,
  sync,
  connect,
  setOnlineAccount,
  removeAccount,
  reloadThemes,
  openThemesFolder,
} = store

const exportingDiagnostics = ref(false)
const diagnosticsNote = ref('')
async function exportDiagnostics(): Promise<void> {
  exportingDiagnostics.value = true
  diagnosticsNote.value = ''
  try {
    if (await window.kchess.exportDiagnostics()) diagnosticsNote.value = 'Diagnostics exported.'
  } catch (cause) {
    console.warn('[settings] Diagnostics export failed:', cause)
    diagnosticsNote.value = cause instanceof Error ? cause.message : 'Could not export diagnostics.'
  } finally {
    exportingDiagnostics.value = false
  }
}

const confirmClearRuns = ref(false)
async function clearLocalRuns(): Promise<void> {
  await window.kchess.clearRuns()
}
const confirmResetUsage = ref(false)

/** One row per account with anything to report; removed accounts stay listed so past downloads are not hidden. */
const dataRows = computed(() => {
  const known = new Map(
    data.value.accounts.map((account) => [account.username.toLowerCase(), account]),
  )
  const names = new Set([
    ...known.keys(),
    ...Object.keys(usage.report?.accounts ?? {}),
    ...Object.keys(usage.report?.storage ?? {}),
  ])
  return [...names]
    .map((name) => {
      const account = known.get(name)
      const used = usage.usageOf(name)
      const kept = usage.storageOf(name)
      return {
        name,
        label: name ? `@${account?.username ?? name}` : 'App (no account)',
        badge: account
          ? account.connected
            ? 'Connected'
            : 'Friend'
          : name
            ? 'Removed'
            : undefined,
        detail: name ? undefined : 'Sign-in and other requests not tied to one account',
        stored: kept.bytes + kept.cacheBytes,
        games: kept.games,
        downloaded: used.total.bytesIn,
        requests: used.total.requests,
      }
    })
    .filter((row) => known.has(row.name) || row.stored || row.downloaded || row.requests)
    .sort((a, b) => b.downloaded + b.stored - (a.downloaded + a.stored))
})

// The sidebar lists the categories while Settings is open; the one chosen there is the page shown here.
const category = computed(
  () =>
    SETTINGS_SECTIONS.find((section) => section.id === settingsSection.value) ??
    SETTINGS_SECTIONS[0],
)
onMounted(() => void usage.refresh())

const appearanceItems = [
  { label: 'System', value: 'system', icon: 'i-lucide-monitor' },
  { label: 'Light', value: 'light', icon: 'i-lucide-sun' },
  { label: 'Dark', value: 'dark', icon: 'i-lucide-moon' },
] as const
const promotionItems = [
  { label: 'Ask me', value: 'ask' },
  { label: 'Always queen', value: 'queen' },
  { label: 'Queen when premoving', value: 'premove' },
] as const
/** Turn one computer level on or off, keeping ladder order and at least one level. */
function toggleEngineLevel(id: EngineLevel, on: boolean): void {
  if (!settings.value) return
  const current = settings.value.engineLevels
  const next = on ? [...current, id] : current.filter((level) => level !== id)
  if (!next.length) return
  settings.value.engineLevels = ENGINE_LADDER.map((entry) => entry.id).filter((level) =>
    next.includes(level),
  )
}
const engineLevelsAreDefault = computed(
  () => settings.value?.engineLevels.join() === DEFAULT_ENGINE_LEVELS.join(),
)
const coordinateItems = [
  { label: 'Inside', value: 'inside' },
  { label: 'Outside', value: 'outside' },
  { label: 'Hidden', value: 'none' },
] as const

/** The two pickers on the App theme page: one theme for each appearance. */
const themePickers = [
  {
    key: 'lightTheme',
    label: 'Light mode theme',
    dark: false,
  },
  { key: 'darkTheme', label: 'Dark mode theme', dark: true },
] as const

const animationItems = [
  { label: 'Off', value: 'none' },
  { label: 'Fast', value: 'fast' },
  { label: 'Normal', value: 'normal' },
  { label: 'Slow', value: 'slow' },
] as const
const notifyCategories = [
  {
    key: 'notifyOpponentMove',
    title: 'Opponent moves',
  },
  {
    key: 'notifyLowTime',
    title: 'Low on time',
  },
  {
    key: 'notifyGameEvents',
    title: 'Game start and result',
  },
  {
    key: 'notifyComputerMove',
    title: 'Computer moves',
  },
  {
    key: 'notifyChallenges',
    title: 'Challenges',
  },
] as const
const pollItems = [
  { label: 'Off', value: 0 },
  { label: '2 min', value: 2 },
  { label: '5 min', value: 5 },
  { label: '15 min', value: 15 },
  { label: '30 min', value: 30 },
  { label: '1 hour', value: 60 },
]
/** Simple yes/no rows of the Gameplay, Online play and Analysis sections. */
const switchRows = {
  gameplay: [
    {
      key: 'showOpeningName',
      title: 'Show opening names',
    },
    {
      key: 'blindfold',
      title: 'Blindfold',
      hint: 'Hide pieces while playing.',
    },
    {
      key: 'zenMode',
      title: 'Zen mode',
      hint: 'Board and game controls only · Z to toggle.',
    },
  ],
  online: [
    {
      key: 'receiveChallenges',
      title: 'Receive challenges',
    },
    {
      key: 'onlineChat',
      title: 'Game chat',
    },
  ],
  analysis: [
    {
      key: 'cloudEval',
      title: 'Lichess cloud evaluation',
      hint: 'Sends analyzed positions to Lichess. Paused during your online games.',
    },
  ],
} as const

const testNote = ref('')
const skipReasons: Record<NotificationSkip, string> = {
  disabled: 'Notifications are turned off.',
  'category-off': 'That kind of notification is turned off.',
  'window-state': 'Alerts are off for the current window state.',
  unsupported: 'This system does not support desktop notifications.',
  failed: 'The system refused the notification.',
}
async function openSystemSettings(): Promise<void> {
  const opened = await window.kchess.openNotificationSettings().catch((error: unknown) => {
    console.warn('[settings] Could not open notification settings:', error)
    return false
  })
  if (!opened) testNote.value = 'Open your system notification settings and allow KChess there.'
}

const TEST_DELAY_SECONDS = 5
const testPending = ref(false)
/** Delayed so there is time to switch to another app: macOS does not show banners for the app in front. */
async function sendTest(): Promise<void> {
  testPending.value = true
  try {
    for (let left = TEST_DELAY_SECONDS; left > 0; left--) {
      testNote.value = `Switch to another app now… sending in ${left}s`
      await new Promise((resolve) => setTimeout(resolve, 1000))
    }
    testNote.value = ''
    const result = await window.kchess.notify({
      kind: 'test',
      title: 'KChess notifications',
      body: 'This is how game alerts will look. Click it to return to KChess.',
    })
    testNote.value = result.shown
      ? result.via === 'in-app'
        ? 'Shown inside KChess. Switch to another app before it sends to try the system notification.'
        : 'Sent. If nothing appeared, check KChess in your system notification settings and that Focus is off.'
      : `${skipReasons[result.skipped ?? 'unsupported']}${result.error ? ` (${result.error})` : ''}`
  } catch (cause) {
    console.warn('[settings] Test notification failed:', cause)
    testNote.value = cause instanceof Error ? cause.message : 'Could not send the notification.'
  } finally {
    testPending.value = false
  }
}

const mic = useMicrophoneAccess()
const micBadges: Record<
  MicrophoneStatus,
  { label: string; color: 'success' | 'error' | 'neutral' }
> = {
  granted: { label: 'Allowed', color: 'success' },
  denied: { label: 'Blocked', color: 'error' },
  restricted: { label: 'Restricted', color: 'error' },
  'not-determined': { label: 'Not asked yet', color: 'neutral' },
  unknown: { label: 'Unknown', color: 'neutral' },
}
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

// The account stays set after the dialog closes so its text doesn't flicker while it animates out.
const pendingRemoval = ref<LichessAccount | null>(null)
const confirmRemove = ref(false)
function askRemove(account: LichessAccount): void {
  pendingRemoval.value = account
  confirmRemove.value = true
}
const removalCopy = computed(() =>
  pendingRemoval.value?.connected
    ? {
        title: 'Disconnect your Lichess account?',
        label: 'Disconnect',
        text: `@${pendingRemoval.value.username} will be signed out and its synced games removed from this device. Your games on Lichess are not affected.`,
      }
    : {
        title: 'Stop tracking this account?',
        label: 'Stop tracking',
        text: `@${pendingRemoval.value?.username} and its synced games will be removed from this device.`,
      },
)
function removePending(): void {
  if (pendingRemoval.value) void removeAccount(pendingRemoval.value.username)
}
</script>

<template>
  <div>
    <PageHeader :title="category.label" />

    <div class="settings-page">
      <LazyAppUpdateSettings v-if="category.id === 'updates'" />
      <LazyOfflineDownloads v-if="category.id === 'offline'" />
      <section
        v-if="category.id === 'appearance'"
        id="settings-appearance"
        class="card settings-list"
        aria-labelledby="appearance-title"
      >
        <h2 id="appearance-title" class="sr-only">Appearance</h2>
        <div class="setting-row">
          <div class="setting-info">
            <span id="theme-label" class="setting-title">Theme</span>
          </div>
          <UTabs
            v-model="settings.appearance"
            :items="[...appearanceItems]"
            aria-labelledby="theme-label"
            :content="false"
            variant="pill"
            class="setting-control"
            :ui="{ trigger: 'grow' }"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span id="coords-label" class="setting-title">Board coordinates</span>
          </div>
          <UTabs
            v-model="settings.coordinates"
            :items="[...coordinateItems]"
            aria-labelledby="coords-label"
            :content="false"
            variant="pill"
            class="setting-control"
            :ui="{ trigger: 'grow' }"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span id="animation-label" class="setting-title">Piece animation</span>
          </div>
          <UTabs
            v-model="settings.pieceAnimation"
            :items="[...animationItems]"
            aria-labelledby="animation-label"
            :content="false"
            variant="pill"
            class="setting-control"
            :ui="{ trigger: 'grow' }"
          />
        </div>
        <div class="setting-row stacked">
          <div class="setting-info">
            <span id="piece-set-label" class="setting-title">Piece set</span>
          </div>
          <div
            class="theme-grid setting-control"
            role="radiogroup"
            aria-labelledby="piece-set-label"
          >
            <button
              v-for="set in pieceSets"
              :key="set.id"
              type="button"
              role="radio"
              class="theme-option"
              :aria-checked="settings.pieceSet === set.id"
              :style="pieceVars(set.id)"
              @click="settings.pieceSet = set.id"
            >
              <span class="piece-preview" aria-hidden="true">
                <span class="piece-preview-piece" style="background-image: var(--piece-wN)" />
                <span class="piece-preview-piece" style="background-image: var(--piece-bQ)" />
              </span>
              {{ set.name }}
            </button>
          </div>
        </div>
        <div class="setting-row stacked">
          <div class="setting-info">
            <span id="board-theme-label" class="setting-title">Board theme</span>
          </div>
          <div
            class="theme-grid setting-control"
            role="radiogroup"
            aria-labelledby="board-theme-label"
          >
            <button
              v-for="theme in boardThemes"
              :key="theme.id"
              type="button"
              role="radio"
              class="theme-option"
              :class="`board-theme-${theme.id}`"
              :aria-checked="settings.boardTheme === theme.id"
              @click="settings.boardTheme = theme.id"
            >
              <span class="board-swatch" />
              {{ theme.name }}
            </button>
          </div>
        </div>
      </section>

      <section
        v-if="category.id === 'themes'"
        id="settings-themes"
        class="card settings-list"
        aria-labelledby="themes-title"
      >
        <h2 id="themes-title" class="sr-only">App theme</h2>
        <div class="setting-row">
          <div class="setting-info">
            <span id="mode-label" class="setting-title">Appearance</span>
          </div>
          <UTabs
            v-model="settings.appearance"
            :items="[...appearanceItems]"
            aria-labelledby="mode-label"
            :content="false"
            variant="pill"
            class="setting-control"
            :ui="{ trigger: 'grow' }"
          />
        </div>
        <div v-for="picker in themePickers" :key="picker.key" class="setting-row stacked">
          <div class="setting-info">
            <span :id="`${picker.key}-label`" class="setting-title">{{ picker.label }}</span>
          </div>
          <div
            class="theme-grid app-theme-grid setting-control"
            role="radiogroup"
            :aria-labelledby="`${picker.key}-label`"
          >
            <button
              v-for="theme in themeList"
              :key="theme.id"
              type="button"
              role="radio"
              class="theme-option"
              :aria-checked="settings[picker.key] === theme.id"
              @click="settings[picker.key] = theme.id"
            >
              <span
                class="app-theme-preview"
                aria-hidden="true"
                :style="{
                  background: previewColors(theme, picker.dark).bg,
                  borderColor: previewColors(theme, picker.dark).border,
                }"
              >
                <span
                  class="app-theme-card"
                  :style="{ background: previewColors(theme, picker.dark).elevated }"
                >
                  <i :style="{ background: previewColors(theme, picker.dark).text }" />
                  <i :style="{ background: previewColors(theme, picker.dark).primary }" />
                </span>
              </span>
              {{ theme.name }}
              <span v-if="theme.custom" class="theme-tag">custom</span>
            </button>
          </div>
        </div>
        <div class="setting-row stacked">
          <div class="setting-info">
            <span class="setting-title">Your own themes</span>
            <details class="setting-hint">
              <summary>How to add a theme</summary>
              <p>
                Copy <code>_template.json</code> in the themes folder, edit its colors, then reload.
              </p>
              <p v-if="themesDir" class="tabular">{{ themesDir }}</p>
            </details>
          </div>
          <div class="friend-actions">
            <UButton
              variant="outline"
              color="neutral"
              icon="i-lucide-folder-open"
              @click="openThemesFolder"
              >Open themes folder</UButton
            >
            <UButton
              variant="outline"
              color="neutral"
              icon="i-lucide-refresh-cw"
              @click="reloadThemes"
              >Reload themes</UButton
            >
          </div>
          <ul v-if="themeProblems.length" class="theme-problems" role="status">
            <li v-for="problem in themeProblems" :key="problem">{{ problem }}</li>
          </ul>
        </div>
      </section>

      <section
        v-if="category.id === 'gameplay'"
        id="settings-gameplay"
        class="card settings-list"
        aria-labelledby="gameplay-title"
      >
        <h2 id="gameplay-title" class="sr-only">Gameplay</h2>
        <div class="setting-row">
          <div class="setting-info">
            <span id="legal-label" class="setting-title">Show possible moves</span>
          </div>
          <USwitch
            v-model="settings.showLegalMoves"
            aria-labelledby="legal-label"
            class="setting-switch"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span id="premove-label" class="setting-title">Premoves</span>
            <span class="setting-hint">Plays your queued move automatically when legal.</span>
          </div>
          <USwitch
            v-model="settings.premove"
            aria-labelledby="premove-label"
            class="setting-switch"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span id="promotion-label" class="setting-title">Pawn promotion</span>
          </div>
          <UTabs
            v-model="settings.promotion"
            :items="[...promotionItems]"
            aria-labelledby="promotion-label"
            :content="false"
            variant="pill"
            class="setting-control"
            :ui="{ trigger: 'grow' }"
          />
        </div>
        <div class="setting-row stacked">
          <div class="setting-info">
            <span id="engine-levels-label" class="setting-title">Computer levels</span>
            <span class="setting-hint">Approximate engine ratings.</span>
          </div>
          <div class="engine-levels" role="group" aria-labelledby="engine-levels-label">
            <UCheckbox
              v-for="entry in ENGINE_LADDER"
              :key="entry.id"
              :model-value="settings.engineLevels.includes(entry.id)"
              :disabled="
                settings.engineLevels.length === 1 && settings.engineLevels[0] === entry.id
              "
              :label="entry.label"
              :description="entry.elo ? `~${entry.elo}` : 'Full strength'"
              @update:model-value="toggleEngineLevel(entry.id, $event === true)"
            />
          </div>
          <div>
            <UButton
              size="sm"
              variant="ghost"
              color="neutral"
              icon="i-lucide-rotate-ccw"
              :disabled="engineLevelsAreDefault"
              @click="settings.engineLevels = [...DEFAULT_ENGINE_LEVELS]"
              >Reset to default</UButton
            >
          </div>
        </div>
        <div v-for="item in switchRows.gameplay" :key="item.key" class="setting-row">
          <div class="setting-info">
            <span :id="`${item.key}-label`" class="setting-title">{{ item.title }}</span>
            <span v-if="'hint' in item" class="setting-hint">{{ item.hint }}</span>
          </div>
          <USwitch
            v-model="settings[item.key]"
            :aria-labelledby="`${item.key}-label`"
            class="setting-switch"
          />
        </div>
      </section>

      <section
        v-if="category.id === 'gestures'"
        id="settings-gestures"
        class="card settings-list"
        aria-labelledby="gestures-title"
      >
        <h2 id="gestures-title" class="sr-only">Gestures</h2>
        <div class="setting-row">
          <div class="setting-info">
            <span id="swipe-nav-label" class="setting-title">Two-finger swipe</span>
            <span class="setting-hint">Go back and forward with a horizontal swipe.</span>
          </div>
          <USwitch
            v-model="settings.swipeNavigation"
            aria-labelledby="swipe-nav-label"
            class="setting-switch"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span id="swipe-hint-label" class="setting-title">Swipe feedback</span>
            <span class="setting-hint">Show an edge hint that follows the swipe.</span>
          </div>
          <USwitch
            v-model="settings.swipeIndicator"
            aria-labelledby="swipe-hint-label"
            class="setting-switch"
            :disabled="!settings.swipeNavigation"
          />
        </div>
      </section>

      <section
        v-if="category.id === 'online' || category.id === 'analysis'"
        :id="`settings-${category.id}`"
        class="card settings-list"
      >
        <h2 class="sr-only">{{ category.label }}</h2>
        <div v-for="item in switchRows[category.id]" :key="item.key" class="setting-row">
          <div class="setting-info">
            <span :id="`${item.key}-label`" class="setting-title">{{ item.title }}</span>
            <span v-if="'hint' in item" class="setting-hint">{{ item.hint }}</span>
          </div>
          <USwitch
            v-model="settings[item.key]"
            :aria-labelledby="`${item.key}-label`"
            class="setting-switch"
          />
        </div>
        <div v-if="category.id === 'online'" class="setting-row">
          <div class="setting-info">
            <span id="corr-label" class="setting-title">Check correspondence games</span>
            <span class="setting-hint">Only while KChess is open.</span>
          </div>
          <USelect
            v-model="settings.correspondencePoll"
            :items="pollItems"
            aria-labelledby="corr-label"
            class="setting-control"
          />
        </div>
      </section>

      <section
        v-if="category.id === 'sound'"
        id="settings-sound"
        class="card settings-list"
        aria-labelledby="sound-title"
      >
        <h2 id="sound-title" class="sr-only">Sound</h2>
        <div class="setting-row">
          <div class="setting-info">
            <span id="sounds-label" class="setting-title">Game sounds</span>
          </div>
          <USwitch
            v-model="settings.soundEnabled"
            aria-labelledby="sounds-label"
            class="setting-switch"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span id="volume-label" class="setting-title">Volume</span>
            <span class="setting-hint tabular">{{ Math.round(settings.soundVolume * 100) }}%</span>
          </div>
          <USlider
            v-model="settings.soundVolume"
            class="setting-control"
            :min="0"
            :max="1"
            :step="0.05"
            :disabled="!settings.soundEnabled"
            aria-labelledby="volume-label"
          />
        </div>
      </section>

      <section
        v-if="category.id === 'notifications'"
        id="settings-notifications"
        class="card settings-list"
        aria-labelledby="notifications-title"
      >
        <h2 id="notifications-title" class="sr-only">Notifications</h2>
        <div class="setting-row">
          <div class="setting-info">
            <span id="notify-label" class="setting-title">Desktop notifications</span>
          </div>
          <USwitch
            v-model="settings.notificationsEnabled"
            aria-labelledby="notify-label"
            class="setting-switch"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span id="notify-active-label" class="setting-title">While I'm using KChess</span>
          </div>
          <USwitch
            v-model="settings.notifyActive"
            aria-labelledby="notify-active-label"
            class="setting-switch"
            :disabled="!settings.notificationsEnabled"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span id="notify-background-label" class="setting-title"
              >While KChess is in the background</span
            >
          </div>
          <USwitch
            v-model="settings.notifyBackground"
            aria-labelledby="notify-background-label"
            class="setting-switch"
            :disabled="!settings.notificationsEnabled"
          />
        </div>
        <div v-for="item in notifyCategories" :key="item.key" class="setting-row">
          <div class="setting-info">
            <span :id="`${item.key}-label`" class="setting-title">{{ item.title }}</span>
          </div>
          <USwitch
            v-model="settings[item.key]"
            :aria-labelledby="`${item.key}-label`"
            class="setting-switch"
            :disabled="!settings.notificationsEnabled"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span id="notify-sound-label" class="setting-title">System sound</span>
            <span class="setting-hint">For system notifications.</span>
          </div>
          <USwitch
            v-model="settings.notifySound"
            aria-labelledby="notify-sound-label"
            class="setting-switch"
            :disabled="!settings.notificationsEnabled"
          />
        </div>
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-title">Try it</span>
            <span class="setting-hint" role="status">{{
              testNote || 'Sends a sample in 5 seconds.'
            }}</span>
          </div>
          <div class="friend-actions">
            <UButton
              variant="ghost"
              color="neutral"
              icon="i-lucide-settings-2"
              @click="openSystemSettings"
              >System settings</UButton
            >
            <UButton
              variant="outline"
              color="neutral"
              icon="i-lucide-bell-ring"
              :disabled="testPending"
              @click="sendTest"
              >Send test</UButton
            >
          </div>
        </div>
      </section>

      <section
        v-if="category.id === 'voice'"
        id="settings-voice"
        class="card settings-list"
        aria-labelledby="voice-title"
      >
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
      <LazyVoiceHistory v-if="category.id === 'voice'" />

      <LazyEngineSettings v-if="category.id === 'engine'" />

      <section
        v-if="category.id === 'accounts'"
        id="settings-accounts"
        class="card"
        aria-labelledby="own-title"
      >
        <div class="card-header">
          <div>
            <h2 id="own-title" class="sr-only">My Lichess accounts</h2>
          </div>
          <UButton
            :variant="connectedAccounts.length ? 'outline' : 'solid'"
            :color="connectedAccounts.length ? 'neutral' : 'primary'"
            icon="i-lucide-link"
            :loading="busy"
            @click="connect"
            >{{
              connectedAccounts.length ? 'Connect another account' : 'Connect my account'
            }}</UButton
          >
        </div>
        <p v-if="connectedAccounts.length" class="section-hint">
          To add another account, sign in to it on lichess.org first.
        </p>
        <UEmpty
          v-if="!connectedAccounts.length"
          variant="naked"
          size="sm"
          icon="i-lucide-user-x"
          title="No account connected"
        />
        <AccountRow
          v-for="account in connectedAccounts"
          :key="account.username"
          :account="account"
          :busy="busy"
          :active="connectedAccounts.length > 1 && account.username === activeOnlineAccount"
          :can-activate="connectedAccounts.length > 1 && account.username !== activeOnlineAccount"
          @sync="sync(account.username)"
          @activate="setOnlineAccount(account.username)"
          @remove="askRemove(account)"
        />
      </section>

      <section
        v-if="category.id === 'data'"
        id="settings-data"
        class="card"
        aria-labelledby="data-title"
      >
        <div class="card-header">
          <div>
            <h2 id="data-title" class="sr-only">Data &amp; storage</h2>
          </div>
          <UButton
            variant="outline"
            color="neutral"
            icon="i-lucide-rotate-ccw"
            :disabled="!usage.totalRequests"
            @click="confirmResetUsage = true"
            >Reset counters</UButton
          >
        </div>
        <div class="data-totals">
          <div>
            <span class="stat-label">Downloaded</span>
            <strong class="tabular">{{ formatBytes(usage.totalDownloaded) }}</strong>
            <span class="muted text-xs"
              >{{ formatCount(usage.totalRequests) }} requests{{
                usage.report?.since
                  ? ` since ${new Date(usage.report.since).toLocaleDateString()}`
                  : ''
              }}</span
            >
          </div>
          <div>
            <span class="stat-label">Stored games &amp; profiles</span>
            <strong class="tabular">{{ formatBytes(usage.totalStored) }}</strong>
          </div>
          <div>
            <span class="stat-label">Database file</span>
            <strong class="tabular">{{ formatBytes(usage.report?.dbBytes ?? 0) }}</strong>
          </div>
        </div>
        <div v-if="dataRows.length" class="table-scroll">
          <table class="data-table">
            <thead>
              <tr>
                <th scope="col">Account</th>
                <th scope="col" class="num">Stored here</th>
                <th scope="col" class="num">Downloaded</th>
                <th scope="col" class="num">Requests</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in dataRows" :key="row.name">
                <td>
                  <strong>{{ row.label }}</strong>
                  <UBadge v-if="row.badge" variant="soft" size="sm" color="neutral" class="ml-2">{{
                    row.badge
                  }}</UBadge>
                  <span v-if="row.detail" class="sub">{{ row.detail }}</span>
                </td>
                <td class="num tabular">
                  {{ formatBytes(row.stored) }}
                  <span class="sub">{{ formatCount(row.games) }} games</span>
                </td>
                <td class="num tabular">{{ formatBytes(row.downloaded) }}</td>
                <td class="num tabular">{{ formatCount(row.requests) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <UEmpty
          v-else
          variant="naked"
          size="sm"
          icon="i-lucide-database"
          title="Nothing counted yet"
        />
      </section>
      <section
        v-if="category.id === 'data'"
        id="settings-puzzles"
        class="card"
        aria-labelledby="local-puzzles-title"
      >
        <div class="card-header">
          <div>
            <h2 id="local-puzzles-title" class="section-title">Puzzles &amp; practice</h2>
          </div>
          <UButton
            variant="outline"
            color="neutral"
            icon="i-lucide-eraser"
            @click="confirmClearRuns = true"
            >Clear local scores</UButton
          >
        </div>
        <UButton
          class="mt-3"
          variant="outline"
          color="neutral"
          icon="i-lucide-download"
          @click="store.jumpToSection('offline')"
          >Offline downloads</UButton
        >
      </section>
      <section
        v-if="category.id === 'data'"
        class="card settings-list"
        aria-labelledby="diagnostics-title"
      >
        <div class="setting-row">
          <div class="setting-info">
            <h2 id="diagnostics-title" class="setting-title">Diagnostics</h2>
            <p class="setting-hint">Exports logs without tokens, games or account databases.</p>
          </div>
          <UButton
            variant="outline"
            color="neutral"
            :loading="exportingDiagnostics"
            @click="exportDiagnostics"
            >Export diagnostics</UButton
          >
        </div>
        <p v-if="diagnosticsNote" role="status" class="setting-hint">{{ diagnosticsNote }}</p>
      </section>
    </div>

    <ConfirmDialog
      v-model:open="confirmClearRuns"
      title="Clear your local scores?"
      description="This deletes the Storm, Streak, Rush and practice scores stored on this computer. Nothing on Lichess is affected."
      confirm-label="Clear"
      color="error"
      @confirm="clearLocalRuns"
    />
    <ConfirmDialog
      v-model:open="confirmResetUsage"
      title="Reset the data counters?"
      description="This only clears the download counts shown here. Your stored games and accounts are not touched."
      confirm-label="Reset"
      @confirm="usage.reset()"
    />
    <ConfirmDialog
      v-model:open="confirmRemove"
      :title="removalCopy.title"
      :description="removalCopy.text"
      :confirm-label="removalCopy.label"
      color="error"
      @confirm="removePending"
    />
  </div>
</template>

<style scoped>
.engine-levels {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(12rem, 1fr));
  gap: 12px 24px;
}
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
