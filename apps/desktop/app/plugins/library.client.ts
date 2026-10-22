import { flushSavedDocuments, loadLibrary } from '../utils/library'
import { flushViewPreferences } from '../composables/useViewPreference'

declare global {
  interface Window {
    __kchessQuitDispose?: () => void
  }
}

/** Stores start from the core's library, so it is loaded before the app mounts. */
export default defineNuxtPlugin(async () => {
  // Client plugins re-execute on HMR: each run would otherwise add another
  // prepare-quit listener and the duplicate would answer the same token twice,
  // surfacing a spurious 'No matching quit request' for the second reply.
  window.__kchessQuitDispose?.()
  window.__kchessQuitDispose = undefined
  window.__kchessQuitDispose = window.kchess?.onPrepareQuit((token) => {
    void Promise.all([flushSavedDocuments(), flushViewPreferences()])
      .then(() => window.kchess?.completeQuit(token))
      .catch((cause) => {
        console.warn('[library] Preparing to quit failed:', cause)
        return window.kchess?.completeQuit(token, false)
      })
      .catch((cause) => {
        console.warn('[library] Reporting quit failure failed:', cause)
      })
  })
  const hot = (import.meta as { hot?: { dispose(callback: () => void): void } }).hot
  hot?.dispose(() => {
    window.__kchessQuitDispose?.()
    window.__kchessQuitDispose = undefined
  })
  await loadLibrary()
})
