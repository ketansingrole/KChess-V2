<script setup lang="ts">
import { useKChessStore } from '../../stores/kchess'
import { previewColors } from '../../utils/themes'
import { appearanceItems, themePickers } from './options'

const store = useKChessStore()
const { settings, themeList, themeProblems, themesDir } = storeToRefs(store)
const { reloadThemes, openThemesFolder } = store
</script>

<template>
  <section id="settings-themes" class="card settings-list" aria-labelledby="themes-title">
    <h2 id="themes-title" class="sr-only">App theme</h2>
    <div class="setting-row">
      <div class="setting-info">
        <span id="mode-label" class="setting-title">Appearance</span>
      </div>
      <UTabs
        v-model="settings.appearance"
        :items="[...appearanceItems]"
        aria-labelledby="mode-label"
        :content="false"
        variant="pill"
        class="setting-control"
        :ui="{ trigger: 'grow' }"
      />
    </div>
    <div v-for="picker in themePickers" :key="picker.key" class="setting-row stacked">
      <div class="setting-info">
        <span :id="`${picker.key}-label`" class="setting-title">{{ picker.label }}</span>
      </div>
      <div
        class="theme-grid app-theme-grid setting-control"
        role="radiogroup"
        :aria-labelledby="`${picker.key}-label`"
      >
        <button
          v-for="theme in themeList"
          :key="theme.id"
          type="button"
          role="radio"
          class="theme-option"
          :aria-checked="settings[picker.key] === theme.id"
          @click="settings[picker.key] = theme.id"
        >
          <span
            class="app-theme-preview"
            aria-hidden="true"
            :style="{
              background: previewColors(theme, picker.dark).bg,
              borderColor: previewColors(theme, picker.dark).border,
            }"
          >
            <span
              class="app-theme-card"
              :style="{ background: previewColors(theme, picker.dark).elevated }"
            >
              <i :style="{ background: previewColors(theme, picker.dark).text }" />
              <i :style="{ background: previewColors(theme, picker.dark).primary }" />
            </span>
          </span>
          {{ theme.name }}
          <span v-if="theme.custom" class="theme-tag">custom</span>
        </button>
      </div>
    </div>
    <div class="setting-row stacked">
      <div class="setting-info">
        <span class="setting-title">Your own themes</span>
        <details class="setting-hint">
          <summary>How to add a theme</summary>
          <p>
            Copy <code>_template.json</code> in the themes folder, edit its colors, then reload.
          </p>
          <p v-if="themesDir" class="tabular">{{ themesDir }}</p>
        </details>
      </div>
      <div class="friend-actions">
        <UButton
          variant="outline"
          color="neutral"
          icon="i-lucide-folder-open"
          @click="openThemesFolder"
          >Open themes folder</UButton
        >
        <UButton variant="outline" color="neutral" icon="i-lucide-refresh-cw" @click="reloadThemes"
          >Reload themes</UButton
        >
      </div>
      <ul v-if="themeProblems.length" class="theme-problems" role="status">
        <li v-for="problem in themeProblems" :key="problem">{{ problem }}</li>
      </ul>
    </div>
  </section>
</template>
