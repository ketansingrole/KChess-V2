import { registerPersistenceFlush } from '../../utils/library'
import { computed, watch, onScopeDispose, type Ref } from 'vue'
import type { AppData, Settings } from '@kchess/contracts/types'

/** `shown` moved from `before` onto `next`: fields still as they were in `before` take `next`'s. */
function rebase(shown: Settings, before: Settings, next: Settings): Settings {
  const merged = { ...shown } as Record<string, unknown>
  const previous = before as unknown as Record<string, unknown>
  for (const [key, value] of Object.entries(next))
    if (JSON.stringify(merged[key]) === JSON.stringify(previous[key])) merged[key] = value
  return merged as unknown as Settings
}

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
  /** Counts saves as they start and finish: settings read before one may predate it. */
  let revision = 0
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
    revision++
    try {
      // Only what this window changed from `base` is written (`docs/shared-core.md`).
      const saved = await window.kchess.saveSettings(sent, base)
      data.value = { ...data.value, settings: saved }
      // Adopt the stored (normalised) copy, keeping only fields edited while the request was in
      // flight: an untouched field must not be written back over another window's change.
      if (settings.value)
        settings.value =
          settingsSignature.value === sentSignature
            ? { ...saved }
            : rebase(settings.value, sent, saved)
      if (saved.enginePath !== previousEngine) void refreshEngine()
    } catch (cause) {
      console.warn('[settings] saving settings failed:', cause)
      fail(cause)
      // Show what is actually stored rather than a change that did not stick.
      if (data.value && settingsSignature.value === sentSignature)
        settings.value = { ...data.value.settings }
      if (strict) throw cause
    } finally {
      revision++
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
    if (settings.value && stored) settings.value = rebase(settings.value, stored, next)
    if (data.value) data.value = { ...data.value, settings: next }
  }
  return { settingsDirty, save, flushSettings, adoptStored, revision: () => revision }
}
