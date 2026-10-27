import { effectScope, ref, watch, type Ref } from 'vue'

/**
 * View preferences (tabs, filters, toggles, recent items) are kept by the core
 * (`viewPreferences` / `setViewPreference`), so every KChess app on this profile opens a page as
 * it was left. Earlier releases kept them in `localStorage`; a key still there moves to the core
 * the first time a page reads it.
 *
 * Each name has one shared, deeply reactive value: every component that asks for it sees the
 * same ref, as `useLocalStorage` gave them, and changing it (or something inside it) saves it.
 * Another app's change reaches it when the preferences are read again (`profile:changed`), unless
 * this one changed it since.
 */

let saved: Record<string, unknown> = {}
const values = new Map<string, Ref<unknown>>()
/** How each shown value takes the core's value of its name (`undefined`: none is saved). */
const adopters = new Map<string, (value: unknown) => void>()
/** Each name's value as last taken from the core, so taking it is not saved back. */
const adopted = new Map<string, string>()
/** Local changes made so far, and the latest one of each name, so an older read never undoes it. */
let changes = 0
const changedAt = new Map<string, number>()
let reads = 0
/** Each name's latest save, so a fast run of changes reaches the core in order. */
const pending = new Map<string, Promise<void>>()
/** Each name's latest value the core refused, retried before quitting until a later save succeeds. */
const failed = new Map<string, unknown>()
/**
 * Earlier releases' `localStorage` text, by name, until the core has saved the preference: a save
 * that fails leaves it to be moved again at the next start.
 */
const legacy = new Map<string, string>()
// The watchers outlive whichever component first asked for a preference.
let scope = effectScope(true)

/**
 * Read the saved preferences; the app waits for this before it mounts (`plugins/`), and reads them
 * again when another app changed them. Names changed here since the read began keep their value.
 */
export async function loadViewPreferences(): Promise<void> {
  const read = ++reads
  const since = changes
  let next: Record<string, unknown>
  try {
    next = (await window.kchess.viewPreferences?.()) ?? {}
  } catch (cause) {
    console.warn('[view-preferences] Loading saved view preferences failed:', cause)
    return
  }
  if (read !== reads) return
  for (const key of new Set([...Object.keys(saved), ...Object.keys(next)])) {
    if (pending.has(key) || failed.has(key) || (changedAt.get(key) ?? 0) > since) continue
    if (key in next) saved[key] = next[key]
    else delete saved[key]
    adopters.get(key)?.(saved[key])
  }
}

/** Forget every preference held in memory (tests start each case afresh). */
export function resetViewPreferences(): void {
  scope.stop()
  scope = effectScope(true)
  saved = {}
  values.clear()
  adopters.clear()
  adopted.clear()
  changedAt.clear()
  pending.clear()
  failed.clear()
  legacy.clear()
}

/** How an earlier release's `localStorage` text is read; `useLocalStorage` chose by the default. */
export type LegacyRead<T> = (text: string) => T

export interface ViewPreferenceOptions<T> {
  /** Read stored objects that may be null (`StorageSerializers.object`). */
  legacy?: LegacyRead<T>
  /** Fill fields a saved object lacks from the default (`useLocalStorage`'s `mergeDefaults`). */
  mergeDefaults?: boolean
}

/** `StorageSerializers.object`'s reading: JSON text. */
export const readJson = <T>(text: string): T => JSON.parse(text) as T

/** Removes an earlier release's copy once the core holds the preference, unless it changed. */
function forgetLegacy(key: string): void {
  const text = legacy.get(key)
  if (text === undefined) return
  legacy.delete(key)
  try {
    if (localStorage.getItem(key) === text) localStorage.removeItem(key)
  } catch (cause) {
    console.warn('[view-preferences] An earlier view preference could not be removed:', key, cause)
  }
}

function legacyValue<T>(key: string, fallback: T, read?: LegacyRead<T>): T | undefined {
  let text: string | null
  try {
    text = localStorage.getItem(key)
  } catch (cause) {
    console.warn('[view-preferences] Earlier view preferences could not be read:', cause)
    return undefined
  }
  if (text === null) return undefined
  // Kept until the core saves it; an unreadable one goes now.
  legacy.set(key, text)
  try {
    if (read) return read(text)
    if (typeof fallback === 'string' || fallback === null || fallback === undefined)
      return text as T
    if (typeof fallback === 'number') return Number.parseFloat(text) as T
    if (typeof fallback === 'boolean') return (text === 'true') as T
    return JSON.parse(text) as T
  } catch (cause) {
    console.warn('[view-preferences] An earlier view preference could not be read:', key, cause)
    forgetLegacy(key)
    return undefined
  }
}

function plain(value: unknown): unknown {
  return value === undefined ? null : (JSON.parse(JSON.stringify(value)) as unknown)
}

function save(key: string, value: unknown): void {
  const body = plain(value)
  changedAt.set(key, ++changes)
  const previous = pending.get(key) ?? Promise.resolve()
  const next = previous
    .then(() => window.kchess.setViewPreference?.(key, body))
    .then(() => {
      failed.delete(key)
      forgetLegacy(key)
      if (body === null) delete saved[key]
      else saved[key] = body
    })
    .catch((cause: unknown) => {
      console.warn('[view-preferences] Saving a view preference failed:', key, cause)
      failed.set(key, body)
    })
  pending.set(key, next)
  void next.then(() => {
    if (pending.get(key) === next) pending.delete(key)
  })
}

async function settle(): Promise<void> {
  while (pending.size) await Promise.all(pending.values())
}

/**
 * Wait for every preference change to be saved (before quitting), trying those the core refused
 * once more; rejects if any is still unsaved.
 */
export async function flushViewPreferences(): Promise<void> {
  await settle()
  if (!failed.size) return
  for (const [key, body] of [...failed]) save(key, body)
  await settle()
  if (failed.size)
    throw new Error(`View preferences could not be saved: ${[...failed.keys()].join(', ')}`)
}

/** The preference `key` (a `kchess:` name), or `fallback` until it is changed. */
export function useViewPreference<T>(
  key: string,
  fallback: T,
  options: ViewPreferenceOptions<T> = {},
): Ref<T> {
  const existing = values.get(key)
  if (existing) return existing as Ref<T>
  const shown = (stored: T): T =>
    options.mergeDefaults &&
    stored &&
    typeof stored === 'object' &&
    fallback &&
    typeof fallback === 'object' &&
    !Array.isArray(fallback)
      ? ({ ...(plain(fallback) as object), ...(stored as object) } as T)
      : stored
  let initial: T
  let imported = false
  if (key in saved) initial = saved[key] as T
  else {
    const earlier = legacyValue(key, fallback, options.legacy)
    imported = earlier !== undefined
    initial = imported ? (earlier as T) : (plain(fallback) as T)
  }
  const value = ref(shown(initial)) as Ref<T>
  values.set(key, value as Ref<unknown>)
  adopters.set(key, (stored) => {
    const next = shown((stored === undefined ? plain(fallback) : stored) as T)
    const text = JSON.stringify(plain(next))
    if (text === JSON.stringify(plain(value.value))) return
    adopted.set(key, text)
    value.value = next
  })
  scope.run(() =>
    watch(
      value,
      (next) => {
        const taken = adopted.get(key)
        adopted.delete(key)
        if (taken !== JSON.stringify(plain(next))) save(key, next)
      },
      { deep: true },
    ),
  )
  if (imported) save(key, value.value)
  return value
}

/** Forget a preference so its page's default applies again. */
export function forgetViewPreference(key: string): void {
  values.delete(key)
  adopters.delete(key)
  delete saved[key]
  try {
    localStorage.removeItem(key)
  } catch (cause) {
    console.warn('[view-preferences] An earlier view preference could not be removed:', key, cause)
  }
  save(key, null)
}
