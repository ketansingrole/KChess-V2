import { computed, ref, type Ref } from 'vue'
import type { Settings, EngineStatus } from '../../../../../core/src/contracts/types'

/** Engine selection and installation state have one renderer owner across routes. */
export function useEngineSettings(options: {
  settings: Ref<Settings | null>
  run: <T>(action: () => Promise<T>) => Promise<T | undefined>
  info: (message: string) => void
  save: () => Promise<void>
}) {
  const { settings, run, info, save } = options
  const engineReady = ref(false)
  /** True until the first engine check lands, and while a re-check runs. */
  const engineChecking = ref(true)
  const engineInfo = ref<EngineStatus | null>(null)
  async function refreshEngine(): Promise<void> {
    engineChecking.value = true
    try {
      const result = await window.kchess.engineStatus()
      engineInfo.value = result
      engineReady.value = result.ready
    } finally {
      engineChecking.value = false
    }
  }
  /** Which Stockfish is in use, in words; empty until the first check lands. */
  const engineName = computed(() => {
    const status = engineInfo.value
    if (!status?.ready) return ''
    if (status.bundled) return 'Bundled Stockfish'
    return status.path === status.managed.path
      ? `Downloaded Stockfish${status.managed.version ? ` ${status.managed.version}` : ''}`
      : 'Your Stockfish'
  })
  async function recheckEngine(): Promise<void> {
    await refreshEngine().catch((cause: unknown) => {
      console.warn('[engine-settings] rechecking engine failed:', cause)
      engineReady.value = false
    })
  }
  async function chooseEngine(): Promise<void> {
    const path = await window.kchess.chooseEngine()
    if (path && settings.value) {
      settings.value.enginePath = path
      await save()
    }
  }
  async function useBundledEngine(): Promise<void> {
    if (!settings.value) return
    settings.value.enginePath = ''
    await save()
  }
  /**
   * Check for the latest native Stockfish and download it if needed. A first
   * download becomes the engine in use; updating an existing copy leaves the
   * current choice alone.
   */
  async function installEngine(): Promise<void> {
    const hadDownload = engineInfo.value?.managed.installed ?? false
    const result = await run(() => window.kchess.installEngine())
    if (!result || !settings.value) return
    if (!result.updated) {
      info(`You already have the latest Stockfish (${result.version}).`)
      await refreshEngine()
      return
    }
    if (hadDownload) {
      await refreshEngine()
      info(`Stockfish updated to ${result.version}.`)
      return
    }
    settings.value.enginePath = result.path
    await save()
    info(`Stockfish ${result.version} downloaded.`)
  }
  async function useDownloadedEngine(): Promise<void> {
    const path = engineInfo.value?.managed.path
    if (!path || !settings.value) return
    settings.value.enginePath = path
    await save()
  }
  /** Delete the downloaded engine; if it was in use, fall back to the bundled one. */
  async function deleteEngine(): Promise<void> {
    const managedPath = engineInfo.value?.managed.path
    const deleted = await run(async () => {
      await window.kchess.deleteEngine()
      return true
    })
    if (!deleted) return
    if (settings.value && settings.value.enginePath === managedPath) {
      settings.value.enginePath = ''
      await save()
    } else await refreshEngine()
    info('Downloaded Stockfish deleted.')
  }
  return {
    engineReady,
    engineChecking,
    engineInfo,
    engineName,
    refreshEngine,
    recheckEngine,
    chooseEngine,
    useBundledEngine,
    installEngine,
    useDownloadedEngine,
    deleteEngine,
  }
}
