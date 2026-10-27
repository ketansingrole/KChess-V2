import { registerPersistenceFlush } from '../../utils/library'
import { computed, watch, onScopeDispose, type Ref } from 'vue'
import type { AppData, Settings } from '@kchess/contracts/types'

/** Serialize settings writes and keep edits made while a prior write is pending. */
export function useSettingsPersistence(options: {
  settings: Ref<Settings | null>
  data: Ref<AppData | null>
  fail: (cause: unknown) => void
  refreshEngine: () => Promise<void>
}) {
  const { settings, data, fail, refreshEngine } = options
  // Both serializations are cached by their computed: a save reads the dirty flag several times,
  // and each read used to stringify the whole settings object twice.
  const settingsSignature = computed(() => (settings.value ? JSON.stringify(settings.value) : ''))
  const storedSignature = computed(() =>
    data.value?.settings ? JSON.stringify(data.value.settings) : '',
  )
  const settingsDirty = computed(
    () =>
      Boolean(settings.value && data.value) && settingsSignature.value !== storedSignature.value,
  )
  // Settings save themselves. Saves run one at a time, and each is a no-op
  // when nothing changed, so explicit calls (engine actions) and the debounced
  // watcher below never double-write.
  let saveChain: Promise<void> = Promise.resolve()
  function save(strict = false): Promise<void> {
    const result = saveChain.then(() => persistSettings(strict))
    saveChain = result.then(
      () => {},
      () => {},
    )
    return result
  }
  async function persistSettings(strict: boolean): Promise<void> {
    if (!settings.value || !data.value || !settingsDirty.value) return
    // A reactive array cannot cross IPC (structured clone): send a plain copy of the list.
    const sent = { ...settings.value, engineLevels: [...settings.value.engineLevels] }
    const sentSignature = JSON.stringify(sent)
    const stored = data.value.settings
    const base = { ...stored, engineLevels: [...stored.engineLevels] }
    const previousEngine = stored.enginePath
    try {
      // Only what this window changed from `base` is written (`docs/shared-core.md`).
      const saved = await window.kchess.saveSettings(sent, base)
      data.value = { ...data.value, settings: saved }
      // Keep edits made while the request was in flight; only adopt the stored
      // (normalised) copy when the form still matches what was sent.
      if (settings.value && settingsSignature.value === sentSignature) settings.value = { ...saved }
      if (saved.enginePath !== previousEngine) void refreshEngine()
    } catch (cause) {
      console.warn('[settings] saving settings failed:', cause)
      fail(cause)
      // Show what is actually stored rather than a change that did not stick.
      if (data.value && settingsSignature.value === sentSignature)
        settings.value = { ...data.value.settings }
      if (strict) throw cause
    }
  }
  let timer: ReturnType<typeof setTimeout> | undefined
  function saveSoon(): void {
    clearTimeout(timer)
    timer = setTimeout(() => void save(), 400)
  }
  const unregister = registerPersistenceFlush(() => {
    clearTimeout(timer)
    return save(true)
  })
  onScopeDispose(() => {
    clearTimeout(timer)
    unregister()
  })
  watch(
    settings,
    () => {
      if (settingsDirty.value) void saveSoon()
    },
    { deep: true },
  )
  /** Write pending edits at once, e.g. when the window is closing. */
  function flushSettings(): void {
    if (settingsDirty.value) void save()
  }
  /**
   * Another window stored `next`: settings this window has not changed show it; its own edits
   * stay, still to be saved.
   */
  function adoptStored(next: Settings): void {
    const stored = data.value?.settings
    if (settings.value && stored) {
      const shown = { ...settings.value } as Record<string, unknown>
      const before = stored as unknown as Record<string, unknown>
      for (const [key, value] of Object.entries(next))
        if (JSON.stringify(shown[key]) === JSON.stringify(before[key])) shown[key] = value
      settings.value = shown as unknown as Settings
    }
    if (data.value) data.value = { ...data.value, settings: next }
  }
  return { settingsDirty, save, flushSettings, adoptStored }
}
