<script setup lang="ts">
import { computed } from 'vue'
import type { EngineLevel } from '@kchess/contracts/types'
import { DEFAULT_ENGINE_LEVELS, ENGINE_LADDER } from '@kchess/rules/engineLevels'
import { useKChessStore } from '../../stores/kchess'
import { gameplaySwitches, promotionItems } from './options'

const store = useKChessStore()
const { settings } = storeToRefs(store)

/** Turn one computer level on or off, keeping ladder order and at least one level. */
function toggleEngineLevel(id: EngineLevel, on: boolean): void {
  if (!settings.value) return
  const current = settings.value.engineLevels
  const next = on ? [...current, id] : current.filter((level) => level !== id)
  if (!next.length) return
  settings.value.engineLevels = ENGINE_LADDER.map((entry) => entry.id).filter((level) =>
    next.includes(level),
  )
}
const engineLevelsAreDefault = computed(
  () => settings.value?.engineLevels.join() === DEFAULT_ENGINE_LEVELS.join(),
)
</script>

<template>
  <section id="settings-gameplay" class="card settings-list" aria-labelledby="gameplay-title">
    <h2 id="gameplay-title" class="sr-only">Gameplay</h2>
    <div class="setting-row">
      <div class="setting-info">
        <span id="legal-label" class="setting-title">Show possible moves</span>
      </div>
      <USwitch
        v-model="settings.showLegalMoves"
        aria-labelledby="legal-label"
        class="setting-switch"
      />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="premove-label" class="setting-title">Premoves</span>
        <span class="setting-hint">Plays your queued move automatically when legal.</span>
      </div>
      <USwitch v-model="settings.premove" aria-labelledby="premove-label" class="setting-switch" />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="promotion-label" class="setting-title">Pawn promotion</span>
      </div>
      <UTabs
        v-model="settings.promotion"
        :items="[...promotionItems]"
        aria-labelledby="promotion-label"
        :content="false"
        variant="pill"
        class="setting-control"
        :ui="{ trigger: 'grow' }"
      />
    </div>
    <div class="setting-row stacked">
      <div class="setting-info">
        <span id="engine-levels-label" class="setting-title">Computer levels</span>
        <span class="setting-hint">Approximate engine ratings.</span>
      </div>
      <div class="engine-levels" role="group" aria-labelledby="engine-levels-label">
        <UCheckbox
          v-for="entry in ENGINE_LADDER"
          :key="entry.id"
          :model-value="settings.engineLevels.includes(entry.id)"
          :disabled="settings.engineLevels.length === 1 && settings.engineLevels[0] === entry.id"
          :label="entry.label"
          :description="entry.elo ? `~${entry.elo}` : 'Full strength'"
          @update:model-value="toggleEngineLevel(entry.id, $event === true)"
        />
      </div>
      <div>
        <UButton
          size="sm"
          variant="ghost"
          color="neutral"
          icon="i-lucide-rotate-ccw"
          :disabled="engineLevelsAreDefault"
          @click="settings.engineLevels = [...DEFAULT_ENGINE_LEVELS]"
          >Reset to default</UButton
        >
      </div>
    </div>
    <div v-for="item in gameplaySwitches" :key="item.key" class="setting-row">
      <div class="setting-info">
        <span :id="`${item.key}-label`" class="setting-title">{{ item.title }}</span>
        <span v-if="'hint' in item" class="setting-hint">{{ item.hint }}</span>
      </div>
      <USwitch
        v-model="settings[item.key]"
        :aria-labelledby="`${item.key}-label`"
        class="setting-switch"
      />
    </div>
  </section>
</template>

<style scoped>
.engine-levels {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(12rem, 1fr));
  gap: 12px 24px;
}
</style>
