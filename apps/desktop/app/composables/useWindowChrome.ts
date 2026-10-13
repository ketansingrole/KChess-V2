import { computed } from 'vue'

/** Match the native title bar and controls in both startup and the workspace. */
export function useWindowChrome() {
  const electron = () =>
    typeof navigator !== 'undefined' && navigator.userAgent.includes('Electron')
  const mac = () =>
    typeof navigator !== 'undefined' && /Mac/.test(navigator.platform || navigator.userAgent)
  return {
    isMac: computed(() => electron() && mac()),
    isFrameless: computed(() => electron() && !mac()),
  }
}
