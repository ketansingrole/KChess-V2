import { describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { storeToRefs } from 'pinia'
import AppWorkspace from '../../app/components/AppWorkspace.vue'
import StartupScreen from '../../app/components/StartupScreen.vue'
import { useKChessStore } from '../../app/stores/kchess'
import { DEFAULT_SETTINGS } from '@kchess/contracts/defaultSettings'
import type { DesktopApi } from '../../contracts/types'
import { deferred, desktop } from './fixtures'

type LoadData = Awaited<ReturnType<DesktopApi['loadData']>>

function appData(): LoadData {
  return {
    settings: { ...DEFAULT_SETTINGS },
    accounts: [{ username: 'Alice', connected: true }],
    gameCount: 0,
  }
}

/** Channels AppWorkspace's mounted side effects need beyond the kchess store itself. */
function workspaceBackend(overrides: Partial<DesktopApi> = {}) {
  return desktop({
    onReviewUpdate: () => vi.fn(),
    onReviewStatus: () => vi.fn(),
    reviewStatus: async () => ({ waiting: 0 }),
    onAppUpdate: () => vi.fn(),
    appUpdateStatus: async () => ({
      phase: 'idle',
      currentVersion: '2026.10.0',
      canCheck: false,
      canInstall: false,
    }),
    onChallenges: () => vi.fn(),
    onLobbyState: () => vi.fn(),
    onOngoingChanged: () => vi.fn(),
    challenges: async () => [],
    ongoingGames: async () => [],
    stayConnected: async () => {},
    ...overrides,
  })
}

describe('startup retry loading state', () => {
  it('tracks init in flight and single-flights concurrent retries', async () => {
    const gate = deferred<LoadData>()
    const loadData = vi.fn(() => gate.promise)
    workspaceBackend({ loadData })
    const store = useKChessStore()
    const { initializing } = storeToRefs(store)

    expect(initializing.value).toBe(false)
    const first = store.init()
    expect(initializing.value).toBe(true)
    // A double-clicked retry while loading must not start a second load.
    await store.init()
    expect(loadData).toHaveBeenCalledTimes(1)

    gate.resolve(appData())
    await first
    expect(initializing.value).toBe(false)
    expect(store.ready).toBe(true)
    expect(loadData).toHaveBeenCalledTimes(1)
  })

  it('clears initializing after a failure so a later retry succeeds', async () => {
    workspaceBackend({ loadData: async () => Promise.reject(new Error('start failed')) })
    const store = useKChessStore()
    const { initializing } = storeToRefs(store)

    await store.init()
    expect(store.ready).toBe(false)
    expect(store.error).toBe('start failed')
    expect(initializing.value).toBe(false)

    workspaceBackend({})
    await store.init()
    expect(store.ready).toBe(true)
    expect(store.error).toBe('')
    expect(initializing.value).toBe(false)
  })

  it('shows the retry button as loading while a retry is in flight', async () => {
    vi.stubGlobal('useRouter', () => ({ options: { history: { state: {} } }, go: vi.fn() }))
    workspaceBackend({ loadData: async () => Promise.reject(new Error('start failed')) })
    // Full mount keeps StartupScreen real; only the sidebar needs a stub so its
    // slot-prop templates (`{ state }`) have something to destructure.
    const wrapper = mount(AppWorkspace, {
      global: {
        // AppWorkspace relies on Nuxt auto-imports; register the one under test.
        components: { StartupScreen },
        stubs: {
          // The auto-stub provides no slot props; the sidebar templates destructure `state`.
          USidebar: {
            template:
              '<div><slot name="header" /><slot name="default" state="expanded" /><slot name="footer" state="expanded" /></div>',
          },
        },
      },
    })
    await flushPromises()

    const splash = wrapper.getComponent(StartupScreen)
    expect(splash.props('error')).toBe('start failed')
    expect(splash.props('loading')).toBe(false)

    const gate = deferred<LoadData>()
    workspaceBackend({ loadData: () => gate.promise })
    splash.vm.$emit('retry')
    await flushPromises()
    expect(wrapper.getComponent(StartupScreen).props('loading')).toBe(true)

    gate.resolve(appData())
    await flushPromises()
    expect(wrapper.findComponent(StartupScreen).exists()).toBe(false)
    expect(useKChessStore().ready).toBe(true)
    wrapper.unmount()
  })
})
