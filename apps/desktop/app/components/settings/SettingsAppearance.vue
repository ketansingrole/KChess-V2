<script setup lang="ts">
import { computed } from 'vue'
import { useKChessStore } from '../../stores/kchess'
import { boardThemes } from '../../utils/boards'
import { pieceSets, pieceVars } from '../../utils/pieces'
import { animationItems, appearanceItems, coordinateItems } from './options'
import { useInterfaceZoom } from '../../composables/useInterfaceZoom'
import { useWindowChrome } from '../../composables/useWindowChrome'
import { ZOOM_PERCENTS, type InterfaceZoom } from '../../../contracts/interfaceZoom'

const store = useKChessStore()
const { settings } = storeToRefs(store)
const { zoom, setZoom } = useInterfaceZoom()
const { isMac } = useWindowChrome()
const zoomItems = computed(() => [
  { label: `Auto (${zoom.value.autoPercent}%)`, value: 'auto' as InterfaceZoom },
  ...ZOOM_PERCENTS.map((percent) => ({ label: `${percent}%`, value: percent as InterfaceZoom })),
])
const zoomKey = computed(() => (isMac.value ? '⌘' : 'Ctrl'))
function chooseZoom(setting: InterfaceZoom): void {
  void store.runAction(() => setZoom(setting))
}
</script>

<template>
  <section id="settings-appearance" class="card settings-list" aria-labelledby="appearance-title">
    <h2 id="appearance-title" class="sr-only">Appearance</h2>
    <div class="setting-row">
      <div class="setting-info">
        <span id="theme-label" class="setting-title">Theme</span>
      </div>
      <UTabs
        v-model="settings.appearance"
        :items="[...appearanceItems]"
        aria-labelledby="theme-label"
        :content="false"
        variant="pill"
        class="setting-control"
        :ui="{ trigger: 'grow' }"
      />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="zoom-label" class="setting-title">Interface size</span>
        <span class="setting-hint">{{ zoomKey }} + and {{ zoomKey }} − adjust it anywhere.</span>
      </div>
      <USelect
        :model-value="zoom.setting"
        :items="zoomItems"
        aria-labelledby="zoom-label"
        class="setting-control"
        @update:model-value="chooseZoom"
      />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="coords-label" class="setting-title">Board coordinates</span>
      </div>
      <UTabs
        v-model="settings.coordinates"
        :items="[...coordinateItems]"
        aria-labelledby="coords-label"
        :content="false"
        variant="pill"
        class="setting-control"
        :ui="{ trigger: 'grow' }"
      />
    </div>
    <div class="setting-row">
      <div class="setting-info">
        <span id="animation-label" class="setting-title">Piece animation</span>
      </div>
      <UTabs
        v-model="settings.pieceAnimation"
        :items="[...animationItems]"
        aria-labelledby="animation-label"
        :content="false"
        variant="pill"
        class="setting-control"
        :ui="{ trigger: 'grow' }"
      />
    </div>
    <div class="setting-row stacked">
      <div class="setting-info">
        <span id="piece-set-label" class="setting-title">Piece set</span>
      </div>
      <div class="theme-grid setting-control" role="radiogroup" aria-labelledby="piece-set-label">
        <button
          v-for="set in pieceSets"
          :key="set.id"
          type="button"
          role="radio"
          class="theme-option"
          :aria-checked="settings.pieceSet === set.id"
          :style="pieceVars(set.id)"
          @click="settings.pieceSet = set.id"
        >
          <span class="piece-preview" aria-hidden="true">
            <span class="piece-preview-piece" style="background-image: var(--piece-wN)" />
            <span class="piece-preview-piece" style="background-image: var(--piece-bQ)" />
          </span>
          {{ set.name }}
        </button>
      </div>
    </div>
    <div class="setting-row stacked">
      <div class="setting-info">
        <span id="board-theme-label" class="setting-title">Board theme</span>
      </div>
      <div class="theme-grid setting-control" role="radiogroup" aria-labelledby="board-theme-label">
        <button
          v-for="theme in boardThemes"
          :key="theme.id"
          type="button"
          role="radio"
          class="theme-option"
          :class="`board-theme-${theme.id}`"
          :aria-checked="settings.boardTheme === theme.id"
          @click="settings.boardTheme = theme.id"
        >
          <span class="board-swatch" />
          {{ theme.name }}
        </button>
      </div>
    </div>
  </section>
</template>
