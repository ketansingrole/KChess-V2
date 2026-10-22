<script setup lang="ts">
/** The flip-board button every board offers; `variant` and `size` follow the buttons beside it. */
withDefaults(
  defineProps<{
    /** What flipping does, for its tooltip and accessible name. */
    label?: string
    /** The page's key for it, shown in the tooltip. */
    shortcut?: string
    variant?: 'ghost' | 'outline'
    size?: 'xs' | 'sm' | 'md'
  }>(),
  { label: 'Flip board', shortcut: undefined, variant: 'ghost', size: 'sm' },
)
const emit = defineEmits<{ flip: [] }>()
</script>

<template>
  <UTooltip :text="label" :kbds="shortcut ? [shortcut] : undefined">
    <UButton
      :size="size"
      :variant="variant"
      color="neutral"
      icon="i-lucide-arrow-up-down"
      :aria-label="label"
      @click="emit('flip')"
    />
  </UTooltip>
</template>
