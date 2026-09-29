<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import type { LichessAccount } from '../../src/shared/types'
import { formatBytes, formatCount } from '../utils/format'
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
