<script setup lang="ts">
import { ref } from 'vue'
import { useKChessStore } from '../../stores/kchess'
import { notifyCategories, skipReasons } from './options'

const store = useKChessStore()
const { settings } = storeToRefs(store)

const testNote = ref('')
const TEST_DELAY_SECONDS = 5
const testPending = ref(false)
async function openSystemSettings(): Promise<void> {
  const opened = await window.kchess.openNotificationSettings().catch((error: unknown) => {
    console.warn('[settings] Could not open notification settings:', error)
    return false
  })
  if (!opened) testNote.value = 'Open your system notification settings and allow KChess there.'
}

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
</script>

<template>
  <section
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
</template>
