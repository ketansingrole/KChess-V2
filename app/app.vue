<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useKChessStore } from './stores/kchess'
import { useAppUpdatesStore } from './stores/appUpdates'
import { useReviewStore } from './stores/review'
import { useChallengeStore } from './stores/challenges'

useHead({
  title: 'KChess',
  htmlAttrs: { lang: 'en' },
  link: [{ rel: 'icon', type: 'image/png', href: './icon.png' }],
})

const store = useKChessStore()
const updates = useAppUpdatesStore()
function showUpdates(): void {
  store.settingsSection = 'updates'
  void navigateTo('/settings')
}

// The search box should sit in the middle of the window, but the top bar only spans the area
// right of the sidebar. Tell the CSS how far that area starts from the window's left edge so it
// can shift the box back by half; the observer also follows the sidebar's open/close animation.
const mainEl = ref<HTMLElement | null>(null)
let resizeObserver: ResizeObserver | undefined
function trackMainOffset(): void {
  const main = mainEl.value
  if (main) main.style.setProperty('--main-left', `${main.getBoundingClientRect().left}px`)
}

onMounted(() => {
  const challenges = useChallengeStore()
  void store.init().then(() => {
    if (store.ready) challenges.start()
    return window.kchess.recordPerformance?.('app.ready', performance.now()).catch(() => {})
  })
  void updates.init()
  useReviewStore().listen()
  if (mainEl.value) {
    resizeObserver = new ResizeObserver(trackMainOffset)
    resizeObserver.observe(mainEl.value)
    trackMainOffset()
  }
})
onUnmounted(() => {
  resizeObserver?.disconnect()
  useChallengeStore().stop()
  store.dispose()
  updates.dispose()
})
const {
  ready,
  sidebarOpen,
  navItems,
  userMenuItems,
  sidebarUserLabel,
  busy,
  error,
  searchOpen,
  searchGroups,
  confirmation,
  zenActive,
} = storeToRefs(store)
const { toggleSearch } = store

// Questions raised outside a page (e.g. starting a game from the palette mid-game). The dialog keeps
// its own copy so the text survives the store clearing the question while the dialog animates out.
const confirmOpen = ref(false)
const asked = ref(confirmation.value)
watch(confirmation, (question) => {
  if (!question) return
  asked.value = question
  confirmOpen.value = true
})
watch(confirmOpen, (open) => {
  if (!open) confirmation.value = null
})

// macOS Electron uses a hiddenInset titlebar: traffic lights float over the
// sidebar header (x12, wider on recent macOS). The sidebar toggle is fixed to
// the window (OpenChamber-style) so it never migrates while the sidebar
// animates: on macOS it holds a constant x84, clear of the lights in both
// sidebar states. Collapsed, it floats over the main topbar, which reserves
// matching space beside the 64px rail (see .mac-toggle-spacer).
// Windows/Linux use OpenChamber-style frameless chrome: no OS title bar, the
// renderer draws its own minimize/maximize/close buttons in the topbar.
// (Gated on the Electron user agent so plain browser previews are unaffected.)
const isMac = computed(
  () =>
    typeof navigator !== 'undefined' &&
    /Mac/.test(navigator.platform || navigator.userAgent) &&
    navigator.userAgent.includes('Electron'),
)
const isFrameless = computed(
  () =>
    typeof navigator !== 'undefined' &&
    navigator.userAgent.includes('Electron') &&
    !/Mac/.test(navigator.platform || navigator.userAgent),
)

function toggleMaximize(event?: MouseEvent): void {
  if (!isFrameless.value) return
  // Double-clicking a button (e.g. rapidly pressing maximize) must not toggle twice.
  if (event?.target instanceof HTMLElement && event.target.closest('button, input, a')) return
  try {
    const api = (
      window as unknown as { kchess?: { windowToggleMaximize?: () => Promise<unknown> } }
    ).kchess
    void api?.windowToggleMaximize?.()?.catch(() => {})
  } catch {
    /* browser preview has no window controls */
  }
}
</script>

<template>
  <UApp>
    <div
      class="app-shell"
      :class="{ 'is-mac': isMac, 'is-frameless': isFrameless, zen: zenActive }"
    >
      <USidebar
        v-model:open="sidebarOpen"
        collapsible="icon"
        rail
        :menu="{ ui: { content: 'max-w-[85vw] sm:max-w-xs' } }"
        :ui="{ header: 'h-(--topbar-height) min-h-(--topbar-height)' }"
      >
        <template #header>
          <div class="sidebar-titlebar">
            <div class="sidebar-toggle-wrap">
              <UTooltip :text="sidebarOpen ? 'Hide sidebar' : 'Show sidebar'">
                <UButton
                  icon="i-lucide-panel-left"
                  color="neutral"
                  variant="ghost"
                  size="sm"
                  :aria-label="sidebarOpen ? 'Hide sidebar' : 'Show sidebar'"
                  class="sidebar-toggle"
                  @click="sidebarOpen = !sidebarOpen"
                />
              </UTooltip>
            </div>
          </div>
        </template>
        <template #default="{ state }">
          <UNavigationMenu
            :key="state"
            :items="navItems"
            orientation="vertical"
            :collapsed="state === 'collapsed'"
            :tooltip="state === 'collapsed'"
            :ui="{ link: 'p-1.5 overflow-hidden' }"
          />
        </template>
        <template #footer="{ state }">
          <UDropdownMenu
            :items="userMenuItems"
            :content="{ align: 'center', side: 'top', sideOffset: 8, collisionPadding: 12 }"
            :ui="{ content: 'w-(--reka-dropdown-menu-trigger-width) min-w-48' }"
          >
            <UButton
              icon="i-lucide-user"
              :label="state === 'expanded' ? sidebarUserLabel : undefined"
              aria-label="Account menu"
              color="neutral"
              variant="ghost"
              :trailing-icon="state === 'expanded' ? 'i-lucide-chevrons-up-down' : undefined"
              square
              block
              class="data-[state=open]:bg-elevated overflow-hidden"
              :ui="{ trailingIcon: 'text-dimmed ms-auto' }"
            />
          </UDropdownMenu>
        </template>
      </USidebar>
      <main ref="mainEl" class="main-area">
        <header class="topbar" @dblclick="toggleMaximize">
          <div class="topbar-left">
            <div
              v-if="isMac && !sidebarOpen"
              class="mac-toggle-spacer hidden lg:block"
              aria-hidden="true"
            />
            <div v-if="!sidebarOpen" class="lg:hidden">
              <UTooltip text="Show sidebar">
                <UButton
                  icon="i-lucide-panel-left"
                  color="neutral"
                  variant="ghost"
                  size="sm"
                  aria-label="Show sidebar"
                  @click="sidebarOpen = true"
                />
              </UTooltip>
            </div>
          </div>
          <div class="topbar-center">
            <UButton
              icon="i-lucide-search"
              variant="outline"
              color="neutral"
              size="sm"
              class="search-trigger"
              aria-label="Search pages and actions"
              @click="toggleSearch"
            >
              <span class="search-trigger-label">Search…</span>
              <span class="search-trigger-keys" aria-hidden="true">
                <UKbd value="meta" size="sm" /><UKbd value="K" size="sm" />
              </span>
            </UButton>
          </div>
          <div class="topbar-right">
            <UButton
              v-if="updates.attention"
              size="sm"
              variant="soft"
              icon="i-lucide-download"
              :label="updates.status?.phase === 'downloaded' ? 'Update ready' : 'Update available'"
              @click="showUpdates"
            />
            <span v-if="busy" class="working" role="status">
              <UIcon name="i-lucide-loader-circle" class="animate-spin" /> Working…
            </span>
            <WindowControls v-if="isFrameless" />
          </div>
        </header>
        <div class="main-content">
          <div v-if="!ready" class="splash">
            <img src="./assets/icon.png" alt="" class="splash-logo" />
            <template v-if="error">
              <h1 class="splash-title">KChess couldn't start</h1>
              <p class="splash-text" role="alert">{{ error }}</p>
              <UButton icon="i-lucide-rotate-cw" @click="store.init()">Try again</UButton>
            </template>
            <template v-else>
              <UIcon name="i-lucide-loader-circle" class="animate-spin splash-spinner" />
              <p class="splash-text">Loading KChess…</p>
            </template>
          </div>
          <div v-else class="page">
            <NuxtPage />
          </div>
        </div>
      </main>
      <UButton
        v-if="zenActive"
        class="zen-exit"
        size="sm"
        variant="soft"
        color="neutral"
        icon="i-lucide-minimize"
        @click="store.toggleSetting('zenMode')"
        >Leave zen mode</UButton
      >
      <UModal
        v-model:open="searchOpen"
        title="Search"
        description="Jump to a page or run an action"
        :ui="{ content: 'max-w-lg' }"
      >
        <template #content>
          <UCommandPalette
            :groups="searchGroups"
            :fuse="{ fuseOptions: { useExtendedSearch: true } }"
            placeholder="Search pages and actions…"
            :close="true"
            class="h-80"
            @update:open="searchOpen = $event"
          />
        </template>
      </UModal>
      <ConfirmDialog
        v-model:open="confirmOpen"
        :title="asked?.title ?? ''"
        :description="asked?.description"
        :confirm-label="asked?.label"
        @confirm="asked?.run()"
      />
    </div>
  </UApp>
</template>
