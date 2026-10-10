<script setup lang="ts">
import { formatGameDate } from '../utils/games'
import { formatBytes, formatCount } from '../utils/format'
import type { LichessAccount } from '@kchess/contracts/types'

const usage = useUsageStore()

defineProps<{
  account: LichessAccount
  busy?: boolean
  /** This is the account that plays online games. */
  active?: boolean
  /** Offer switching online play to this account. */
  canActivate?: boolean
}>()
defineEmits<{ sync: []; remove: []; activate: [] }>()
</script>

<template>
  <div class="account-row">
    <div class="account-id">
      <UAvatar :text="account.username.slice(0, 2).toUpperCase()" size="md" />
      <div class="min-w-0">
        <div class="flex items-center gap-2">
          <strong class="truncate">@{{ account.username }}</strong>
          <UBadge :color="account.connected ? 'success' : 'neutral'" variant="soft" size="sm">{{
            account.connected ? 'Connected' : 'Tracked'
          }}</UBadge>
          <UBadge v-if="active" color="primary" variant="soft" size="sm">Plays online</UBadge>
        </div>
        <div class="muted text-xs">
          {{
            account.lastSyncedAt
              ? `Last synced ${formatGameDate(account.lastSyncedAt)}`
              : 'Not synced yet'
          }}
        </div>
        <div class="muted text-xs tabular">
          {{ formatCount(usage.storageOf(account.username).games) }} games ·
          {{
            formatBytes(
              usage.storageOf(account.username).bytes +
                usage.storageOf(account.username).cacheBytes,
            )
          }}
          stored · {{ formatBytes(usage.usageOf(account.username).total.bytesIn) }} downloaded
        </div>
      </div>
    </div>
    <div class="toolbar-row">
      <UButton
        v-if="canActivate"
        size="sm"
        variant="ghost"
        color="neutral"
        icon="i-lucide-gamepad-2"
        :disabled="busy"
        @click="$emit('activate')"
        >Use for online play</UButton
      >
      <UButton
        size="sm"
        variant="outline"
        color="neutral"
        icon="i-lucide-refresh-cw"
        :disabled="busy"
        @click="$emit('sync')"
        >Sync</UButton
      >
      <UButton
        size="sm"
        variant="ghost"
        color="error"
        :icon="account.connected ? 'i-lucide-unlink' : 'i-lucide-user-minus'"
        @click="$emit('remove')"
        >{{ account.connected ? 'Disconnect' : 'Stop tracking' }}</UButton
      >
    </div>
  </div>
</template>
