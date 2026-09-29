<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import type { LichessAccount } from '../../src/shared/types'
import { formatBytes, formatCount } from '../utils/format'
import { pieceSets, pieceVars } from '../utils/pieces'
import { previewColors } from '../utils/themes'
import type { NotificationSkip } from '../../src/shared/types'
import { SETTINGS_SECTIONS } from '../stores/kchess'

const store = useKChessStore()
const usage = useUsageStore()
const {
  busy,
  data,
  settings,
  engineReady,
  engineInfo,
  connectedAccounts,
  activeOnlineAccount,
  settingsSection,
  themeList,
  themeProblems,
  themesDir,
} = storeToRefs(store)
const {
  boardThemes,
  useBundledEngine,
  useDownloadedEngine,
  installEngine,
  deleteEngine,
  chooseEngine,
  sync,
  connect,
  setOnlineAccount,
  removeAccount,
  reloadThemes,
  openThemesFolder,
} = store

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

const confirmDeleteEngine = ref(false)
/** Which Stockfish is in use: the bundled one (no path), the one KChess downloaded, or a chosen file. */
const engineSource = computed<'bundled' | 'downloaded' | 'custom'>(() => {
  const path = settings.value?.enginePath
  if (!path) return 'bundled'
  return path === engineInfo.value?.managed.path ? 'downloaded' : 'custom'
})
const downloadedDescription = computed(() => {
  const managed = engineInfo.value?.managed
  if (managed?.installed) return managed.version ? `Version ${managed.version}` : 'Installed'
  return engineInfo.value?.canDownload === false
    ? 'Downloading is available on macOS.'
    : 'Native build from the official Stockfish release, verified by checksum.'
})
const customDescription = computed(() =>
  engineSource.value === 'custom'
    ? settings.value!.enginePath
    : 'Pick a Stockfish executable already on this computer.',
)

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
    hint: 'Used when the app is light.',
    dark: false,
  },
  { key: 'darkTheme', label: 'Dark mode theme', hint: 'Used when the app is dark.', dark: true },
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
    hint: 'Your Lichess opponent has moved and it is your turn.',
  },
  {
    key: 'notifyLowTime',
    title: 'Low on time',
    hint: 'Your clock is running low in an online game.',
  },
  {
    key: 'notifyGameEvents',
    title: 'Game start and result',
    hint: 'An online game begins, or ends with a win, loss or draw.',
  },
  {
    key: 'notifyComputerMove',
    title: 'Computer moves',
    hint: 'Stockfish has replied in a game against the computer.',
  },
] as const

const testNote = ref('')
const skipReasons: Record<NotificationSkip, string> = {
  disabled: 'Notifications are turned off.',
  'category-off': 'That kind of notification is turned off.',
  'window-state': 'Alerts are off for the current window state.',
  unsupported: 'This system does not support desktop notifications.',
  failed: 'The system refused the notification.',
}
async function openSystemSettings(): Promise<void> {
  const opened = await window.kchess.openNotificationSettings().catch(() => false)
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
    testNote.value = cause instanceof Error ? cause.message : 'Could not send the notification.'
  } finally {
    testPending.value = false
  }
}

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
    <PageHeader :title="category.label" subtitle="Settings · changes save automatically" />

    <div class="settings-page">
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
            <span class="setting-hint">Light, dark, or follow your system.</span>
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
            <span class="setting-hint">Where the a–h and 1–8 labels appear.</span>
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
            <span class="setting-hint">How quickly pieces slide to their squares.</span>
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
            <span class="setting-hint">The look of the pieces, from Lichess's open sets.</span>
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
            <span class="setting-hint">The colors or texture of the board squares.</span>
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
            <span class="setting-hint"
              >Light, dark, or follow your system. Each has its own theme below.</span
            >
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
            <span class="setting-hint">{{ picker.hint }}</span>
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
            <span class="setting-hint"
              >Drop a JSON file in the themes folder to add one. Open it, copy
              <code>_template.json</code>, change the colors, then choose Reload. Only
              <code>bg</code>, <code>text</code> and <code>primary</code> are required; the rest are
              worked out from them. A theme with one palette is used in both modes.</span
            >
            <span v-if="themesDir" class="setting-hint tabular">{{ themesDir }}</span>
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
            <span class="setting-hint"
              >Dot the squares a piece can move to when you select it.</span
            >
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
            <span class="setting-hint"
              >Queue a move while it is your opponent's turn; it plays automatically if legal.</span
            >
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
            <span class="setting-hint">Which piece a pawn becomes on the last rank.</span>
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
            <span class="setting-hint">Moves, captures and low-time warnings.</span>
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
            <span class="setting-hint"
              >Get an alert from your system when something happens in a game.</span
            >
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
            <span class="setting-hint"
              >An alert appears inside the window, for example when you are on another page.</span
            >
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
            <span class="setting-hint"
              >A system notification when another app is in front or KChess is minimized or hidden.
              Click it to come back.</span
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
            <span class="setting-hint">{{ item.hint }}</span>
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
            <span class="setting-hint"
              >Let your system play its notification sound. KChess's own game sounds are under
              Sound.</span
            >
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
              testNote ||
              'Sends a sample in 5 seconds. Stay here to see the in-app alert, or switch to another app for the system one.'
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
        v-if="category.id === 'engine'"
        id="settings-engine"
        class="card settings-list"
        aria-labelledby="engine-title"
      >
        <h2 id="engine-title" class="sr-only">Chess engine</h2>
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-title">Status</span>
            <span class="setting-hint">Choose which Stockfish plays the computer games.</span>
          </div>
          <div>
            <UBadge
              :color="engineReady ? 'success' : 'warning'"
              variant="soft"
              :icon="engineReady ? 'i-lucide-circle-check' : 'i-lucide-triangle-alert'"
              >{{ engineReady ? 'Ready' : 'Not found' }}</UBadge
            >
          </div>
        </div>
        <div class="engine-options">
          <EngineOption
            title="Bundled Stockfish 19"
            description="Included with KChess. Works offline on every platform."
            :active="engineSource === 'bundled'"
          >
            <UButton
              v-if="engineSource !== 'bundled'"
              size="sm"
              variant="outline"
              color="neutral"
              :disabled="busy"
              @click="useBundledEngine"
              >Use</UButton
            >
          </EngineOption>
          <EngineOption
            title="Downloaded Stockfish"
            :description="downloadedDescription"
            :active="engineSource === 'downloaded'"
          >
            <UButton
              v-if="engineInfo?.managed.installed && engineSource !== 'downloaded'"
              size="sm"
              variant="outline"
              color="neutral"
              :disabled="busy"
              @click="useDownloadedEngine"
              >Use</UButton
            >
            <UButton
              v-if="engineInfo?.canDownload"
              size="sm"
              variant="outline"
              color="neutral"
              icon="i-lucide-download"
              :loading="busy"
              @click="installEngine"
              >{{ engineInfo.managed.installed ? 'Check for update' : 'Download' }}</UButton
            >
            <UButton
              v-if="engineInfo?.managed.installed"
              size="sm"
              variant="ghost"
              color="error"
              icon="i-lucide-trash-2"
              :disabled="busy"
              @click="confirmDeleteEngine = true"
              >Delete</UButton
            >
          </EngineOption>
          <EngineOption
            title="Your own executable"
            :description="customDescription"
            :active="engineSource === 'custom'"
          >
            <UButton
              size="sm"
              variant="outline"
              color="neutral"
              icon="i-lucide-folder-open"
              :disabled="busy"
              @click="chooseEngine"
              >Choose file…</UButton
            >
          </EngineOption>
        </div>
      </section>

      <section
        v-if="category.id === 'accounts'"
        id="settings-accounts"
        class="card"
        aria-labelledby="own-title"
      >
        <div class="card-header">
          <div>
            <h2 id="own-title" class="sr-only">My Lichess accounts</h2>
            <p class="section-hint">
              Accounts you own. Connect each one you want to play online games with from KChess.
            </p>
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
          Lichess connects whichever account is signed in to your browser. To add another account,
          sign in to it on lichess.org first, then choose Connect another account.
        </p>
        <UEmpty
          v-if="!connectedAccounts.length"
          variant="naked"
          size="sm"
          icon="i-lucide-user-x"
          title="No account connected"
          description="You'll be sent to Lichess to approve access."
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
            <p class="section-hint">
              What KChess has downloaded from Lichess for each account, and how much space it takes
              on this computer. Sizes are after decompression. Followed players live on the
              <NuxtLink to="/friends" class="link">Friends</NuxtLink> page.
            </p>
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
            <span class="muted text-xs">Estimated size of the data itself</span>
          </div>
          <div>
            <span class="stat-label">Database file</span>
            <strong class="tabular">{{ formatBytes(usage.report?.dbBytes ?? 0) }}</strong>
            <span class="muted text-xs">Everything KChess keeps on disk</span>
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
          description="Downloads from Lichess are counted from now on and listed here per account."
        />
      </section>
    </div>

    <ConfirmDialog
      v-model:open="confirmResetUsage"
      title="Reset the data counters?"
      description="This only clears the download counts shown here. Your stored games and accounts are not touched."
      confirm-label="Reset"
      @confirm="usage.reset()"
    />
    <ConfirmDialog
      v-model:open="confirmDeleteEngine"
      title="Delete downloaded Stockfish?"
      description="This removes the copy KChess downloaded. The bundled engine and any executable you chose yourself are not touched, and you can download it again later."
      confirm-label="Delete"
      color="error"
      @confirm="deleteEngine"
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
