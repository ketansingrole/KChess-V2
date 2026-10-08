import { loadLibrary } from '../utils/library'

/** Stores start from the core's library, so it is loaded before the app mounts. */
export default defineNuxtPlugin(async () => {
  await loadLibrary()
})
