import { isReactive, nextTick, onScopeDispose, ref, watch, type Ref } from 'vue'
import {
  LEGACY_DOCUMENT_KEYS,
  type LegacyDocuments,
  type LibrarySnapshot,
  type SessionDocuments,
  type SessionKind,
} from '@kchess/rules/library'

const EMPTY: LibrarySnapshot = {
  studies: [],
  games: [],
  mistakes: [],
  sessions: {},
  joinedTournaments: [],
  repertoireMisses: {},
  imported: true,
}
let current: LibrarySnapshot = EMPTY
export const libraryReady = ref(false)
export const libraryLoading = ref(false)
export const libraryError = ref('')

/** The library as loaded at startup; stores copy what they own from it when created. */
export function initialLibrary(): LibrarySnapshot {
  return current
}
/** Tests seed the library stores start from. */
export function setInitialLibrary(snapshot: Partial<LibrarySnapshot> = {}): void {
  current = { ...EMPTY, ...snapshot }
  libraryReady.value = true
  libraryLoading.value = false
  libraryError.value = ''
}

const persistence = new Set<() => Promise<void>>()
export function registerPersistenceFlush(flush: () => Promise<void>): () => void {
  persistence.add(flush)
  return () => {
    persistence.delete(flush)
  }
}
export async function flushSavedDocuments(): Promise<void> {
  // Let reactive game/archive watchers publish the final state before collecting writes.
  await nextTick()
  const results = await Promise.allSettled(
    [...persistence].map((flush) => Promise.resolve().then(flush)),
  )
  const failures: unknown[] = []
  for (const result of results)
    if (result.status === 'rejected') {
      console.warn('[library] Quit flush failed:', result.reason)
      failures.push(result.reason)
    }
  if (failures.length) throw new AggregateError(failures, 'Some changes could not be saved.')
}

/** Load the core's library before the app mounts, handing over documents earlier releases kept here. */
export async function loadLibrary(): Promise<void> {
  if (libraryLoading.value) return
  // libraryReady stays sticky once true: resetting it here would unmount AppWorkspace
  // (app.vue gates it on v-if="libraryReady") and dispose live stores on any reload.
  libraryLoading.value = true
  try {
    let snapshot = await window.kchess.library()
    if (!snapshot.imported) {
      const documents: LegacyDocuments = {}
      for (const key of LEGACY_DOCUMENT_KEYS) {
        const text = localStorage.getItem(key)
        if (text !== null) documents[key] = text
      }
      snapshot = await window.kchess.importLibrary(documents)
      for (const key of Object.keys(documents)) {
        // A damaged core copy must not cause us to delete an earlier renderer recovery copy.
        if (key !== 'kchess:studies:v1' || !snapshot.studyError) localStorage.removeItem(key)
      }
    }
    current = snapshot
    libraryReady.value = true
    libraryError.value = ''
  } catch (cause) {
    console.warn('[library] loading the library failed:', cause)
    libraryError.value = cause instanceof Error ? cause.message : 'The library could not be loaded.'
  } finally {
    libraryLoading.value = false
  }
}

/** Plain data for IPC: reactive proxies cannot be cloned. */
export const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value)) as T

/** A session's source is usually a plain rules snapshot already; only unwrap a reactive one. */
const sessionPayload = <T>(value: T): T => (isReactive(value) ? plain(value) : value)

/**
 * Keep an unfinished session in the core's library: saved shortly after each change, and when the
 * window closes. The returned ref holds the last save error.
 */
export function persistSession<K extends SessionKind>(
  kind: K,
  source: () => SessionDocuments[K],
  changes: () => unknown = source,
  options: { flush?: 'pre' | 'sync'; deep?: boolean } = {},
): Ref<string> & { flush: () => Promise<void> } {
  const error = ref('')
  let timer: ReturnType<typeof setTimeout> | undefined
  const save = async (strict = false): Promise<void> => {
    clearTimeout(timer)
    // Unloaded documents must never be replaced by a store's default session.
    if (!libraryReady.value) return
    try {
      await window.kchess.saveSession(kind, sessionPayload(source()))
      error.value = ''
    } catch (cause) {
      console.warn('[library] saving session failed:', kind, cause)
      error.value =
        cause instanceof Error ? cause.message : 'Automatic saving failed. Export a PGN copy.'
      if (strict) throw cause
    }
  }
  const unregister = registerPersistenceFlush(() => save(true))
  const saveNow = (): void => void save()
  const off = watch(
    changes,
    () => {
      clearTimeout(timer)
      timer = setTimeout(saveNow, 150)
    },
    // Snapshots are built from reactive state while the getter runs, so watching the getter
    // already tracks changes; only a proxy returned as-is (archive identity) needs a deep walk.
    { deep: options.deep ?? false, flush: options.flush ?? 'pre' },
  )
  window.addEventListener('beforeunload', saveNow)
  onScopeDispose(() => {
    saveNow()
    unregister()
    off()
    window.removeEventListener('beforeunload', saveNow)
  })
  return Object.assign(error, { flush: () => save() })
}
