import { beforeEach, afterEach, vi } from 'vitest'
import { createPinia, setActivePinia, storeToRefs, getActivePinia, disposePinia } from 'pinia'
import { useKChessStore } from '../../app/stores/kchess'
import { usePuzzleStore } from '../../app/stores/puzzles'
import { useUsageStore } from '../../app/stores/usage'
import type { DesktopApi } from '../../contracts/types'
import { detachedLibraryApi, resetLibrary } from './testLibrary'
// The renderer loads the rules as WebAssembly; tests use the same rules as a Node module
// (crates/kchess-node/js/rules.ts installs them when it loads).
import '@kchess/native/rules'
import { existsSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

{
  // `pnpm run test:unit` alone (without `build:native`) fails obscurely inside
  // `nativeRules()`; fail here with the fix instead.
  const native = join(
    dirname(fileURLToPath(import.meta.url)),
    '..',
    '..',
    '..',
    '..',
    'crates',
    'kchess-node',
    `kchess-native.${process.platform}-${process.arch}.node`,
  )
  if (!existsSync(native))
    throw new Error(
      `Native rules are not built (${native} is missing). Run \`pnpm run build:native\` first; \`pnpm run check\` and \`check:fast\` order it before unit tests.`,
    )
}
process.env.KCHESS_STORE_DEBUG = '1'

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
