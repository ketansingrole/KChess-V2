import { effectScope, ref, watch, type Ref } from 'vue'

/**
 * View preferences (tabs, filters, toggles, recent items) are kept by the core
 * (`viewPreferences` / `setViewPreference`), so every KChess app on this profile opens a page as
 * it was left. Earlier releases kept them in `localStorage`; a key still there moves to the core
 * the first time a page reads it.
 *
 * Each name has one shared, deeply reactive value: every component that asks for it sees the
 * same ref, as `useLocalStorage` gave them, and changing it (or something inside it) saves it.
 */

let saved: Record<string, unknown> = {}
const values = new Map<string, Ref<unknown>>()
/** Each name's latest save, so a fast run of changes reaches the core in order. */
const pending = new Map<string, Promise<void>>()
// The watchers outlive whichever component first asked for a preference.
let scope = effectScope(true)

/** Read the saved preferences; the app waits for this before it mounts (`plugins/`). */
export async function loadViewPreferences(): Promise<void> {
  try {
    saved = (await window.kchess.viewPreferences?.()) ?? {}
  } catch (cause) {
    console.warn('[view-preferences] Loading saved view preferences failed:', cause)
    saved = {}
  }
}

/** Forget every preference held in memory (tests start each case afresh). */
export function resetViewPreferences(): void {
  scope.stop()
  scope = effectScope(true)
  saved = {}
  values.clear()
  pending.clear()
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

function legacyValue<T>(key: string, fallback: T, read?: LegacyRead<T>): T | undefined {
  let text: string | null
  try {
    text = localStorage.getItem(key)
  } catch (cause) {
    console.warn('[view-preferences] Earlier view preferences could not be read:', cause)
    return undefined
  }
  if (text === null) return undefined
  localStorage.removeItem(key)
  try {
    if (read) return read(text)
    if (typeof fallback === 'string' || fallback === null || fallback === undefined)
      return text as T
    if (typeof fallback === 'number') return Number.parseFloat(text) as T
    if (typeof fallback === 'boolean') return (text === 'true') as T
    return JSON.parse(text) as T
  } catch (cause) {
    console.warn('[view-preferences] An earlier view preference could not be read:', key, cause)
    return undefined
  }
}

function plain(value: unknown): unknown {
  return value === undefined ? null : (JSON.parse(JSON.stringify(value)) as unknown)
}

function save(key: string, value: unknown): void {
  const body = plain(value)
  const previous = pending.get(key) ?? Promise.resolve()
  const next = previous
    .then(() => window.kchess.setViewPreference?.(key, body))
    .catch((cause: unknown) => {
      console.warn('[view-preferences] Saving a view preference failed:', key, cause)
    })
  pending.set(key, next)
  void next.then(() => {
    if (pending.get(key) === next) pending.delete(key)
  })
}

/** Wait for every preference change to be saved (before quitting). */
export async function flushViewPreferences(): Promise<void> {
  while (pending.size) await Promise.all(pending.values())
}

/** The preference `key` (a `kchess:` name), or `fallback` until it is changed. */
export function useViewPreference<T>(
  key: string,
  fallback: T,
  options: ViewPreferenceOptions<T> = {},
): Ref<T> {
  const existing = values.get(key)
  if (existing) return existing as Ref<T>
  let initial: T
  let imported = false
  if (key in saved) initial = saved[key] as T
  else {
    const legacy = legacyValue(key, fallback, options.legacy)
    imported = legacy !== undefined
    initial = imported ? (legacy as T) : (plain(fallback) as T)
  }
  if (
    options.mergeDefaults &&
    initial &&
    typeof initial === 'object' &&
    fallback &&
    typeof fallback === 'object' &&
    !Array.isArray(fallback)
  )
    initial = { ...(plain(fallback) as object), ...(initial as object) } as T
  const value = ref(initial) as Ref<T>
  values.set(key, value as Ref<unknown>)
  scope.run(() => watch(value, (next) => save(key, next), { deep: true }))
  if (imported) save(key, initial)
  return value
}

/** Forget a preference so its page's default applies again. */
export function forgetViewPreference(key: string): void {
  values.delete(key)
  delete saved[key]
  try {
    localStorage.removeItem(key)
  } catch (cause) {
    console.warn('[view-preferences] An earlier view preference could not be removed:', key, cause)
  }
  save(key, null)
}
