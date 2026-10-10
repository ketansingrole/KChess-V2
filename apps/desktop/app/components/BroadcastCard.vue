<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useIntervalFn } from '@vueuse/core'
import type { BroadcastSummary } from '@kchess/contracts/types'

const props = defineProps<{ item: BroadcastSummary; disabled?: boolean }>()
defineEmits<{ select: [] }>()
const failedImage = ref(false)
watch(
  () => props.item.image,
  () => (failedImage.value = false),
)
const now = ref(new Date())
useIntervalFn(() => (now.value = new Date()), 30_000)
const timing = computed(() => {
  if (props.item.ongoing) return 'LIVE'
  if (props.item.section === 'past') return 'Finished'
  if (!props.item.startsAt) return 'Upcoming'
  const minutes = Math.ceil((props.item.startsAt - now.value.getTime()) / 60_000)
  if (minutes <= 0) return 'Starting soon'
  if (minutes < 60) return `in ${minutes} min`
  if (minutes < 24 * 60) return `in ${Math.ceil(minutes / 60)} hours`
  return new Date(props.item.startsAt).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  })
})
</script>

<template>
  <button
    type="button"
    class="broadcast-card"
    :class="{ live: item.ongoing }"
    :disabled="disabled"
    @click="$emit('select')"
  >
    <div class="broadcast-cover">
      <img
        v-if="item.image && !failedImage"
        :src="item.image"
        alt=""
        loading="lazy"
        @error="failedImage = true"
      />
      <UIcon v-else name="i-lucide-radio" class="broadcast-placeholder" aria-hidden="true" />
    </div>
    <div class="broadcast-info">
      <div class="broadcast-round">
        <span class="truncate">{{ item.roundName }}</span>
        <span :class="{ 'broadcast-live': item.ongoing }">{{ timing }}</span>
      </div>
      <h3>{{ item.tourName }}</h3>
      <p v-if="item.players" class="broadcast-description">{{ item.players }}</p>
    </div>
  </button>
</template>

<style scoped>
.broadcast-card {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  width: 100%;
  text-align: left;
  background: var(--ui-bg-elevated);
  border: 1px solid var(--ui-border);
  border-radius: 12px;
  transition: border-color 120ms;
}
.broadcast-card:hover:not(:disabled) {
  border-color: var(--ui-primary);
}
.broadcast-card:focus-visible {
  outline: 2px solid var(--ui-primary);
  outline-offset: 3px;
}
.broadcast-card:disabled {
  opacity: 0.6;
}
.broadcast-card.live {
  border-color: var(--ui-error);
}
.broadcast-cover {
  display: grid;
  place-items: center;
  width: 100%;
  aspect-ratio: 2 / 1;
  background: var(--ui-bg-accented);
}
.broadcast-cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.broadcast-placeholder {
  font-size: 48px;
  color: var(--ui-text-muted);
}
.broadcast-info {
  padding: 16px;
  width: 100%;
}
.broadcast-round {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  color: var(--ui-text-muted);
  font-size: 13px;
  margin-bottom: 8px;
}
.broadcast-live {
  color: var(--ui-error);
  font-weight: 700;
}
h3 {
  font-size: 18px;
  font-weight: 600;
  line-height: 1.4;
}
.broadcast-description {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  color: var(--ui-text-muted);
  font-size: 13px;
  margin-top: 8px;
}
</style>
