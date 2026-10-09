import { beforeEach, afterEach, vi } from 'vitest'
import { createPinia, setActivePinia, storeToRefs, getActivePinia, disposePinia } from 'pinia'
import { useKChessStore } from '../../app/stores/kchess'
import { usePuzzleStore } from '../../app/stores/puzzles'
import { useUsageStore } from '../../app/stores/usage'
import type { DesktopApi } from '../../contracts/types'
import { detachedLibraryApi, resetLibrary } from './testLibrary'
import { installRules } from '../../../../core/src/services/rules'

// The renderer loads the rules as WebAssembly; tests use the same rules as a Node module.
installRules()

vi.mock('../../app/utils/sound', () => ({
  configure: vi.fn(),
  play: vi.fn(async () => {}),
  playMoveSound: vi.fn(),
}))

beforeEach(() => {
  localStorage.clear()
  resetLibrary()
  window.kchess = detachedLibraryApi() as unknown as DesktopApi
  setActivePinia(createPinia())
  vi.stubGlobal('useRoute', () => ({ path: '/online' }))
  vi.stubGlobal('useHead', vi.fn())
  vi.stubGlobal('useToast', () => ({ add: vi.fn() }))
  vi.stubGlobal('navigateTo', vi.fn())
  vi.stubGlobal('storeToRefs', storeToRefs)
  vi.stubGlobal('useKChessStore', useKChessStore)
  vi.stubGlobal('usePuzzleStore', usePuzzleStore)
  vi.stubGlobal('useUsageStore', useUsageStore)
})
afterEach(() => {
  useKChessStore().dispose()
  const pinia = getActivePinia()
  if (pinia) disposePinia(pinia)
  vi.useRealTimers()
  vi.unstubAllGlobals()
})
