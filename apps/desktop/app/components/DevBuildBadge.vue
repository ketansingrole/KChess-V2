<script setup lang="ts">
// Marks a development session (`pnpm run dev`) in the sidebar footer. AppWorkspace imports it only
// under `import.meta.dev`, so production renderers never contain it; `verifyRenderer` checks the
// `data-dev-build` marker is absent. Plain scoped CSS keeps its styles out of the Tailwind build.
defineProps<{ collapsed: boolean }>()
</script>

<template>
  <UTooltip text="Development build · Electron" :disabled="!collapsed">
    <div
      data-dev-build
      class="dev-build"
      :class="{ collapsed }"
      role="note"
      aria-label="Development build · Electron"
    >
      <UIcon name="i-lucide-moon" class="dev-build-icon" />
      <span v-if="!collapsed" class="dev-build-label">Dev · Electron</span>
    </div>
  </UTooltip>
</template>

<style scoped>
.dev-build {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 8px;
  overflow: hidden;
  border-radius: 6px;
  color: var(--ui-warning);
  background: color-mix(in oklab, var(--ui-warning) 12%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in oklab, var(--ui-warning) 30%, transparent);
  font-size: 12px;
  font-weight: 600;
  white-space: nowrap;
}
.dev-build.collapsed {
  justify-content: center;
  padding: 0;
}
.dev-build-icon {
  flex: none;
  width: 16px;
  height: 16px;
}
</style>
