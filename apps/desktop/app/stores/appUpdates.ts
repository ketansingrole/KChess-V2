import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import type { AppUpdateStatus } from '@kchess/contracts/types'

export const useAppUpdatesStore = defineStore('appUpdates', () => {
  const status = ref<AppUpdateStatus | null>(null)
  const error = ref('')
  let offUpdate: (() => void) | undefined
  let revision = 0
  const busy = computed(() => ['checking', 'downloading'].includes(status.value?.phase ?? ''))
  /** A newer release is known: offered, downloading, ready, or a retryable failed download. */
  const attention = computed(() => {
    const phase = status.value?.phase ?? ''
    if (['available', 'downloading', 'downloaded'].includes(phase)) return true
    return phase === 'error' && Boolean(status.value?.version)
  })
  const title = computed(() => {
    switch (status.value?.phase) {
      case 'checking':
        return 'Checking for updates…'
      case 'up-to-date':
        return 'KChess is up to date'
      case 'available':
        return `KChess ${status.value.version} is available`
      case 'downloading':
        return `Downloading KChess ${status.value.version}…`
      case 'downloaded':
        return `KChess ${status.value.version} is ready to install`
      case 'error':
        return 'Update could not finish'
      case 'disabled':
        return 'Updates unavailable in this build'
      default:
        return 'Keep KChess up to date'
    }
  })

  /** Subscribes before reading initial state so newer progress cannot be overwritten. */
  async function init(): Promise<void> {
    if (offUpdate || !window.kchess) return
    offUpdate = window.kchess.onAppUpdate((next) => {
      revision++
      status.value = next
    })
    const started = revision
    try {
      const initial = await window.kchess.appUpdateStatus()
      // A progress event can overtake the initial IPC response.
      if (revision === started) status.value = initial
    } catch (cause) {
      console.warn('[app-updates] reading update status failed:', cause)
      error.value = cause instanceof Error ? cause.message : 'Could not read update status.'
    }
  }

  /** Invokes a main-process update action and surfaces IPC failures to settings. */
  async function action(kind: 'check' | 'download' | 'install' | 'releases'): Promise<void> {
    error.value = ''
    try {
      // Status events own the state: an older invoke response must not undo newer progress.
      if (kind === 'check') await window.kchess.checkAppUpdate()
      else if (kind === 'download') await window.kchess.downloadAppUpdate()
      else if (kind === 'install') await window.kchess.installAppUpdate()
      else await window.kchess.openAppReleases()
    } catch (cause) {
      console.warn('[app-updates] update action failed:', cause)
      error.value = cause instanceof Error ? cause.message : 'Could not complete the update action.'
    }
  }

  /** Releases the status subscription when the app root unmounts. */
  function dispose(): void {
    offUpdate?.()
    offUpdate = undefined
    revision++
  }

  return { status, error, busy, attention, title, init, action, dispose }
})
