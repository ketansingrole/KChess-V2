<script setup lang="ts">
import { computed } from 'vue'

/**
 * Says where a piece of data lives, so nobody has to guess what an action touches:
 * `synced`: doing it here changes your Lichess account; `readonly`: it was fetched from Lichess and nothing
 * done in KChess changes it; `local`: it stays on this computer and is never sent to Lichess.
 */
export type SourceKind = 'synced' | 'readonly' | 'local'

const props = defineProps<{ kind: SourceKind; label?: string }>()

const META = {
  synced: {
    label: 'Synced to Lichess',
    icon: 'i-lucide-cloud-upload',
    color: 'success',
    tip: 'Doing this here changes your Lichess account.',
  },
  readonly: {
    label: 'From Lichess · read-only',
    icon: 'i-lucide-cloud-download',
    color: 'info',
    tip: 'Fetched from Lichess. Nothing you do in KChess changes it.',
  },
  local: {
    label: 'Local only',
    icon: 'i-lucide-hard-drive',
    color: 'neutral',
    tip: 'Stays on this computer. It is never sent to Lichess.',
  },
} as const

const meta = computed(() => META[props.kind])
</script>

<template>
  <UTooltip :text="meta.tip">
    <UBadge
      :color="meta.color"
      :variant="kind === 'local' ? 'outline' : 'soft'"
      :icon="meta.icon"
      size="md"
      class="source-badge"
      :aria-label="`${label ?? meta.label}. ${meta.tip}`"
      >{{ label ?? meta.label }}</UBadge
    >
  </UTooltip>
</template>
