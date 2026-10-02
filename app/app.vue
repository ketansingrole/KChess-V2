<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useKChessStore } from './stores/kchess'
import { useAppUpdatesStore } from './stores/appUpdates'

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
  void store.init()
  void updates.init()
  if (mainEl.value) {
    resizeObserver = new ResizeObserver(trackMainOffset)
    resizeObserver.observe(mainEl.value)
    trackMainOffset()
  }
})
onUnmounted(() => {
  resizeObserver?.disconnect()
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
// sidebar header, so the toggle lives there, right beside them.
// (Gated on the Electron user agent so plain browser previews are unaffected.)
const isMac = computed(
  () =>
    typeof navigator !== 'undefined' &&
    /Mac/.test(navigator.platform || navigator.userAgent) &&
    navigator.userAgent.includes('Electron'),
)
</script>

<template>
  <UApp>
    <div class="app-shell" :class="{ 'is-mac': isMac }">
      <USidebar
        v-model:open="sidebarOpen"
        collapsible="icon"
        rail
        :menu="{ ui: { content: 'max-w-[85vw] sm:max-w-xs' } }"
        :ui="{ header: 'h-(--topbar-height) min-h-(--topbar-height)' }"
      >
        <template #header="{ state }">
          <div class="sidebar-titlebar" :class="state">
            <UTooltip v-if="state === 'expanded'" text="Hide sidebar">
              <UButton
                icon="i-lucide-panel-left"
                color="neutral"
                variant="ghost"
                size="sm"
                aria-label="Hide sidebar"
                class="sidebar-toggle"
                @click="sidebarOpen = false"
              />
            </UTooltip>
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
        <header class="topbar">
          <div class="topbar-left">
            <UTooltip v-if="!sidebarOpen" text="Show sidebar">
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
          </div>
        </header>
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
      </main>
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
