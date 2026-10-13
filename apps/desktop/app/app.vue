<script setup lang="ts">
import { libraryReady, libraryLoading, libraryError, loadLibrary } from './utils/library'
import AppWorkspace from './components/AppWorkspace.vue'
import StartupScreen from './components/StartupScreen.vue'
import { useWindowChrome } from './composables/useWindowChrome'

const { isMac, isFrameless } = useWindowChrome()
</script>

<template>
  <AppWorkspace v-if="libraryReady" />
  <UApp v-else :toaster="null">
    <div class="app-shell" :class="{ 'is-mac': isMac, 'is-frameless': isFrameless }">
      <main class="main-area">
        <header class="topbar">
          <div class="topbar-right"><WindowControls v-if="isFrameless" /></div>
        </header>
        <StartupScreen :error="libraryError" :loading="libraryLoading" @retry="loadLibrary()" />
      </main>
    </div>
  </UApp>
</template>
