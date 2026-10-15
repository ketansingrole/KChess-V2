<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useOnline } from '@vueuse/core'
import { useKChessStore } from '../stores/kchess'
import { useAppUpdatesStore } from '../stores/appUpdates'
import { useReviewStore } from '../stores/review'
import { useChallengeStore } from '../stores/challenges'
import { useQuickPick } from '../composables/useQuickPick'
import { usePageSwipeNavigation } from '../composables/usePageSwipeNavigation'
import { useWindowChrome } from '../composables/useWindowChrome'

usePageSwipeNavigation()

useHead({
  title: 'KChess',
  htmlAttrs: { lang: 'en' },
  meta: [
    {
      name: 'description',
      content:
        'Play chess against Stockfish or on Lichess, practice puzzles, and analyze your games with KChess.',
    },
  ],
  link: [{ rel: 'icon', type: 'image/png', href: './icon.png' }],
})

const online = useOnline()
const store = useKChessStore()
const updates = useAppUpdatesStore()

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
    return window.kchess
      ?.recordPerformance?.('app.ready', performance.now())
      .catch((error: unknown) => {
        console.warn('[app] recording app-ready performance failed:', error)
      })
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
  initializing,
  searchOpen,
  searchQuery,
  confirmation,
  zenActive,
} = storeToRefs(store)
const { groups: searchGroups, mode: searchMode } = useQuickPick(searchQuery)
const challengeStore = useChallengeStore()
/** Play on Lichess carries a dot while a game is on, in the primary colour when it is your move. */
const lichessGame = computed<'turn' | 'live' | null>(() => {
  const live = store.onlinePhase === 'playing'
  if ((live && store.onlineTurn === store.onlineColor) || challengeStore.myTurnCount > 0)
    return 'turn'
  return live ? 'live' : null
})
const sidebarItems = computed(() =>
  navItems.value.map((item) =>
    item.to === '/online' && lichessGame.value
      ? { ...item, class: `nav-game ${lichessGame.value}` }
      : item,
  ),
)
watch(searchOpen, (open) => {
  if (!open) searchQuery.value = ''
})

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
const { isMac, isFrameless } = useWindowChrome()

function toggleMaximize(event?: MouseEvent): void {
  if (!isFrameless.value) return
  // Double-clicking a button (e.g. rapidly pressing maximize) must not toggle twice.
  if (event?.target instanceof HTMLElement && event.target.closest('button, input, a')) return
  try {
    void window.kchess.windowToggleMaximize?.()?.catch((error: unknown) => {
      console.warn('[app] toggling maximize failed:', error)
    })
  } catch (cause) {
    console.warn('[app] toggling maximize failed:', cause)
    /* browser preview has no window controls */
  }
}
</script>

<template>
  <UApp :toaster="null">
    <AppToaster />
    <NuxtRouteAnnouncer />
    <div
      class="app-shell"
      :class="{
        'is-mac': isMac,
        'is-frameless': isFrameless,
        zen: zenActive,
        'search-open': searchOpen,
      }"
    >
      <USidebar
        v-model:open="sidebarOpen"
        collapsible="icon"
        :menu="{
          // The narrow-window drawer is teleported out of .app-shell: carry the platform class so
          // its sidebar toggle also clears the macOS traffic lights.
          ui: { content: ['max-w-[85vw] sm:max-w-xs', isMac && 'is-mac'] },
        }"
        :ui="{
          header: 'h-(--topbar-height) min-h-(--topbar-height)',
          container: 'kchess-sidebar-container',
        }"
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
            :items="sidebarItems"
            orientation="vertical"
            :collapsed="state === 'collapsed'"
            :tooltip="state === 'collapsed'"
            :ui="{ link: 'p-1.5 overflow-hidden' }"
            :class="{ 'nav-collapsed': state === 'collapsed' }"
          >
            <template #item-trailing="{ item }">
              <span
                v-if="item.to === '/online' && lichessGame"
                class="nav-game-dot"
                role="img"
                :aria-label="lichessGame === 'turn' ? 'Your move' : 'Game in progress'"
              />
            </template>
          </UNavigationMenu>
        </template>
        <template #footer="{ state }">
          <div class="flex w-full min-w-0 flex-col gap-1">
            <UTooltip text="Settings" :disabled="state === 'expanded'">
              <UButton
                icon="i-lucide-settings-2"
                :label="state === 'expanded' ? 'Settings' : undefined"
                aria-label="Settings"
                color="neutral"
                variant="ghost"
                :active="store.page === 'settings'"
                block
                class="justify-start overflow-hidden"
                @click="store.choosePage('settings')"
              />
            </UTooltip>
            <UTooltip
              v-if="!store.activeOnlineAccount"
              text="Connect Lichess"
              :disabled="state === 'expanded'"
            >
              <UButton
                icon="i-lucide-log-in"
                :label="state === 'expanded' ? 'Connect Lichess' : undefined"
                aria-label="Connect Lichess"
                color="neutral"
                variant="ghost"
                :disabled="!online || busy"
                block
                class="justify-start overflow-hidden"
                @click="store.connect()"
              />
            </UTooltip>
            <UDropdownMenu
              v-else
              :items="userMenuItems"
              :content="{ align: 'center', side: 'top', sideOffset: 8, collisionPadding: 12 }"
              :ui="{ content: 'w-(--reka-dropdown-menu-trigger-width) min-w-48' }"
            >
              <UButton
                icon="i-lucide-user"
                :label="state === 'expanded' ? sidebarUserLabel : undefined"
                :aria-label="
                  state === 'expanded' ? `${sidebarUserLabel} account menu` : 'Account menu'
                "
                color="neutral"
                variant="ghost"
                :trailing-icon="state === 'expanded' ? 'i-lucide-chevrons-up-down' : undefined"
                square
                block
                class="data-[state=open]:bg-elevated overflow-hidden"
                :ui="{ trailingIcon: 'text-dimmed ms-auto' }"
              />
            </UDropdownMenu>
          </div>
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
            <AppCommandPalette
              v-model:open="searchOpen"
              v-model:query="searchQuery"
              :groups="searchGroups"
              :mode="searchMode"
            />
          </div>
          <div class="topbar-right">
            <UTooltip
              v-if="!online"
              text="Local play, analysis and downloaded content are available."
            >
              <span class="flex items-center gap-1 text-sm text-muted" role="status">
                <UIcon name="i-lucide-wifi-off" /> Offline
              </span>
            </UTooltip>
            <AppUpdateButton />
            <span v-if="busy" class="working" role="status">
              <UIcon name="i-lucide-loader-circle" class="animate-spin" /> Working…
            </span>
            <WindowControls v-if="isFrameless" />
          </div>
        </header>
        <div class="main-content">
          <StartupScreen
            v-if="!ready"
            :error="error"
            :loading="initializing"
            @retry="store.init()"
          />
          <div v-else class="page">
            <PageContent>
              <NuxtPage />
            </PageContent>
          </div>
        </div>
      </main>
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
