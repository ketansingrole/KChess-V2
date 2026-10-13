import { flushSavedDocuments, loadLibrary } from '../utils/library'

/** Stores start from the core's library, so it is loaded before the app mounts. */
export default defineNuxtPlugin(async () => {
  window.kchess?.onPrepareQuit((token) => {
    void flushSavedDocuments()
      .then(() => window.kchess?.completeQuit(token))
      .catch((cause) => {
        console.warn('[library] Preparing to quit failed:', cause)
        return window.kchess?.completeQuit(token, false)
      })
      .catch((cause) => {
        console.warn('[library] Reporting quit failure failed:', cause)
      })
  })
  await loadLibrary()
})
