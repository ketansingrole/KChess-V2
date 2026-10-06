import { onBeforeUnmount, onMounted, ref } from 'vue'
import type { MicrophoneAccess } from '../../src/shared/types'

/** The OS microphone permission, re-checked whenever the window regains focus (e.g. back from System Settings). */
export function useMicrophoneAccess(onFocus?: (access: MicrophoneAccess) => void) {
  const access = ref<MicrophoneAccess | null>(null)
  const note = ref('')

  async function refresh(request = false): Promise<MicrophoneAccess | null> {
    try {
      access.value = (await window.kchess?.microphoneAccess?.(request)) ?? null
    } catch (cause) {
      console.warn('[microphone] checking microphone access failed:', cause)
      access.value = null
    }
    return access.value
  }
  async function openSettings(): Promise<void> {
    const opened = await window.kchess?.openMicrophoneSettings?.().catch((error: unknown) => {
      console.warn('[microphone] opening microphone settings failed:', error)
      return false
    })
    note.value = opened ? '' : 'Open your system privacy settings and allow the microphone there.'
  }
  async function focus(): Promise<void> {
    const value = await refresh()
    if (value) onFocus?.(value)
  }
  onMounted(() => {
    void refresh()
    window.addEventListener('focus', focus)
  })
  onBeforeUnmount(() => window.removeEventListener('focus', focus))
  return { access, note, refresh, openSettings }
}

/** Advice for a macOS dev build, whose microphone permission belongs to the launching app. */
export const LAUNCHER_PERMISSION_HINT =
  'Development build: macOS asks on behalf of the app that started KChess (your terminal or editor). Allow that app under Privacy & Security › Microphone.'
