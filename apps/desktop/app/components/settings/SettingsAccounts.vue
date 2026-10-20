<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import type { LichessAccount } from '@kchess/contracts/types'
import { useKChessStore } from '../../stores/kchess'
import { useUsageStore } from '../../stores/usage'
import AccountRow from '../AccountRow.vue'
import ConfirmDialog from '../ConfirmDialog.vue'

const store = useKChessStore()
const { activeOnlineAccount, busy, connectedAccounts } = storeToRefs(store)
const { connect, setOnlineAccount, removeAccount, sync } = store
// Each row shows what its account keeps and downloaded; this section may be the first to need it.
const usage = useUsageStore()
onMounted(() => void usage.refresh())

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
  <section id="settings-accounts" class="card" aria-labelledby="own-title">
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
        >{{ connectedAccounts.length ? 'Connect another account' : 'Connect my account' }}</UButton
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
    <ConfirmDialog
      v-model:open="confirmRemove"
      :title="removalCopy.title"
      :description="removalCopy.text"
      :confirm-label="removalCopy.label"
      color="error"
      @confirm="removePending"
    />
  </section>
</template>
