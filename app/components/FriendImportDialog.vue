<script setup lang="ts">
import { computed, ref, watch } from 'vue'

const open = defineModel<boolean>('open', { default: false })

const store = useKChessStore()
const friendsStore = useFriendsStore()
const { busy, connectedAccounts } = storeToRefs(store)
const { following, loadingFollowing, followingError } = storeToRefs(friendsStore)

const selected = ref<string[]>([])
const query = ref('')
const showRemoved = ref(false)

// Every time the dialog opens, ask Lichess afresh; nothing is selected by default.
watch(open, (isOpen) => {
  if (!isOpen) return
  selected.value = []
  query.value = ''
  showRemoved.value = false
  void friendsStore.loadFollowing()
})

const users = computed(() => following.value?.users ?? [])
const candidates = computed(() => {
  const needle = query.value.trim().toLowerCase()
  return users.value.filter(
    (user) =>
      !user.alreadyAdded &&
      (showRemoved.value || !user.dismissed) &&
      (!needle || user.username.toLowerCase().includes(needle)),
  )
})
const alreadyCount = computed(() => users.value.filter((user) => user.alreadyAdded).length)
const removedCount = computed(
  () => users.value.filter((user) => user.dismissed && !user.alreadyAdded).length,
)
const problems = computed(() => following.value?.problems ?? [])

function toggle(username: string): void {
  selected.value = selected.value.includes(username)
    ? selected.value.filter((name) => name !== username)
    : [...selected.value, username]
}
const allSelected = computed(
  () =>
    candidates.value.length > 0 &&
    candidates.value.every((u) => selected.value.includes(u.username)),
)
function toggleAll(): void {
  selected.value = allSelected.value ? [] : candidates.value.map((user) => user.username)
}

async function reconnect(): Promise<void> {
  await store.connect()
  await friendsStore.loadFollowing()
}
async function add(): Promise<void> {
  if (await store.importFriends(selected.value)) {
    open.value = false
    void friendsStore.loadProfiles()
    void friendsStore.refreshStatus()
  }
}
</script>

<template>
  <UModal
    v-model:open="open"
    title="Add friends from Lichess"
    description="Players your connected accounts follow on Lichess."
    :ui="{ content: 'max-w-xl' }"
  >
    <template #body>
      <div class="import-body">
        <div v-if="!connectedAccounts.length" class="muted text-sm">
          Connect a Lichess account in Settings first; the list comes from the players it follows.
        </div>

        <UAlert
          v-for="problem in problems"
          :key="problem.account"
          color="warning"
          variant="subtle"
          icon="i-lucide-triangle-alert"
          :title="`@${problem.account}: couldn't read who it follows`"
          :description="problem.message"
          :actions="
            problem.needsReconnect
              ? [{ label: 'Reconnect', color: 'neutral', variant: 'outline', onClick: reconnect }]
              : undefined
          "
        />
        <UAlert
          v-if="followingError"
          color="error"
          variant="subtle"
          icon="i-lucide-circle-alert"
          title="Couldn't reach Lichess"
          :description="followingError"
        />

        <div v-if="loadingFollowing" class="import-status" role="status">
          <UIcon name="i-lucide-loader-circle" class="animate-spin" /> Asking Lichess…
        </div>

        <template v-else-if="following">
          <div class="toolbar-row flex-nowrap">
            <UInput
              v-model="query"
              icon="i-lucide-search"
              placeholder="Filter by name"
              aria-label="Filter followed players"
              autocomplete="off"
              class="flex-1"
            />
            <UButton
              variant="outline"
              color="neutral"
              size="md"
              :disabled="!candidates.length"
              @click="toggleAll"
              >{{ allSelected ? 'Clear' : 'Select all' }}</UButton
            >
          </div>

          <ul v-if="candidates.length" class="import-list" aria-label="Followed players">
            <li v-for="user in candidates" :key="user.username">
              <label class="import-row">
                <input
                  type="checkbox"
                  :checked="selected.includes(user.username)"
                  @change="toggle(user.username)"
                />
                <span class="import-name">
                  <strong class="truncate">@{{ user.username }}</strong>
                  <UBadge v-if="user.title" color="primary" variant="soft" size="sm">{{
                    user.title
                  }}</UBadge>
                  <UBadge v-if="user.dismissed" color="neutral" variant="soft" size="sm"
                    >Removed before</UBadge
                  >
                </span>
                <span class="muted text-xs tabular import-ratings">
                  <template v-if="user.ratings.blitz">Blitz {{ user.ratings.blitz }}</template>
                  <template v-if="user.ratings.rapid"> · Rapid {{ user.ratings.rapid }}</template>
                </span>
                <span v-if="connectedAccounts.length > 1" class="muted text-xs import-via">
                  via {{ user.followedBy.map((name) => `@${name}`).join(', ') }}
                </span>
              </label>
            </li>
          </ul>
          <UEmpty
            v-else
            variant="naked"
            size="sm"
            icon="i-lucide-users"
            :title="users.length ? 'Nobody new to add' : 'No followed players found'"
            :description="
              users.length
                ? 'Everyone they follow is already a friend.'
                : 'Follow players on lichess.org and they will show up here.'
            "
          />

          <p class="muted text-xs">
            {{ alreadyCount }} already added.
            <template v-if="removedCount">
              <button type="button" class="link" @click="showRemoved = !showRemoved">
                {{ showRemoved ? 'Hide' : 'Show' }} {{ removedCount }} you removed earlier
              </button>
              .
            </template>
            Adding friends does not download their games; sync each friend when you want them.
          </p>
        </template>
      </div>
    </template>
    <template #footer>
      <div class="flex w-full items-center justify-between gap-2">
        <span class="muted text-xs tabular">{{ selected.length }} selected</span>
        <div class="flex gap-2">
          <UButton color="neutral" variant="outline" @click="open = false">Cancel</UButton>
          <UButton icon="i-lucide-user-plus" :disabled="!selected.length || busy" @click="add"
            >Add {{ selected.length || '' }}
            {{ selected.length === 1 ? 'friend' : 'friends' }}</UButton
          >
        </div>
      </div>
    </template>
  </UModal>
</template>
