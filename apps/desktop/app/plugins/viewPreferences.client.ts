import { loadViewPreferences } from '../composables/useViewPreference'

/** Pages open as they were left: the saved view preferences load before anything reads one. */
export default defineNuxtPlugin({
  name: 'view-preferences',
  enforce: 'pre',
  async setup() {
    await loadViewPreferences()
  },
})
