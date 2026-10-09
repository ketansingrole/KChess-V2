import { registerPersistenceFlush } from '../../utils/library'
import { computed, watch, onScopeDispose, type Ref } from 'vue'
import type { AppData, Settings } from '../../../../../core/src/contracts/types'

/** Serialize settings writes and keep edits made while a prior write is pending. */
export function useSettingsPersistence(options: {
  settings: Ref<Settings | null>
  data: Ref<AppData | null>
  fail: (cause: unknown) => void
  refreshEngine: () => Promise<void>
}) {
  const { settings, data, fail, refreshEngine } = options
  const settingsDirty = computed(
    () =>
      Boolean(settings.value && data.value) &&
      JSON.stringify(settings.value) !== JSON.stringify(data.value?.settings),
  )
  // Settings save themselves. Saves run one at a time, and each is a no-op
  // when nothing changed, so explicit calls (engine actions) and the debounced
  // watcher below never double-write.
  let saveChain: Promise<void> = Promise.resolve()
  function save(): Promise<void> {
    saveChain = saveChain.then(persistSettings)
    return saveChain
  }
  async function persistSettings(): Promise<void> {
    if (!settings.value || !data.value || !settingsDirty.value) return
    // A reactive array cannot cross IPC (structured clone): send a plain copy of the list.
    const sent = { ...settings.value, engineLevels: [...settings.value.engineLevels] }
    const previousEngine = data.value.settings.enginePath
    try {
      const saved = await window.kchess.saveSettings(sent)
      data.value = { ...data.value, settings: saved }
      // Keep edits made while the request was in flight; only adopt the stored
      // (normalised) copy when the form still matches what was sent.
      if (settings.value && JSON.stringify(settings.value) === JSON.stringify(sent))
        settings.value = { ...saved }
      if (saved.enginePath !== previousEngine) void refreshEngine()
    } catch (cause) {
      console.warn('[settings] saving settings failed:', cause)
      fail(cause)
      // Show what is actually stored rather than a change that did not stick.
      if (data.value) settings.value = { ...data.value.settings }
    }
  }
  let timer: ReturnType<typeof setTimeout> | undefined
  function saveSoon(): void {
    clearTimeout(timer)
    timer = setTimeout(() => void save(), 400)
  }
  const unregister = registerPersistenceFlush(() => {
    clearTimeout(timer)
    return save()
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
  return { settingsDirty, save, flushSettings }
}
