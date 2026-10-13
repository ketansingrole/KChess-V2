import { expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { effectScope } from 'vue'
import App from '../../app/app.vue'
import StartupScreen from '../../app/components/StartupScreen.vue'
import { desktop } from './fixtures'
import { useLocalGameStore } from '../../app/stores/local'
import {
  flushSavedDocuments,
  loadLibrary,
  libraryReady,
  libraryError,
  persistSession,
} from '../../app/utils/library'
import { STANDARD_SETUP } from '@kchess/rules/variant'

it('keeps document stores unmounted after a read failure and restores the saved session on retry', async () => {
  vi.useFakeTimers()
  const { api } = desktop()
  await api.saveSession('local', {
    setup: STANDARD_SETUP,
    moves: ['e2e4'],
    clock: null,
    times: null,
    result: null,
  })
  const read = api.library
  api.library = async () => {
    throw new Error('Temporary read failure')
  }
  await loadLibrary()
  expect(libraryReady.value).toBe(false)
  expect(libraryError.value).toBe('Temporary read failure')
  const started = vi.fn()
  const wrapper = mount(App, {
    global: {
      stubs: {
        UApp: { template: '<div><slot /></div>' },
        UButton: { template: '<button><slot /></button>' },
        UIcon: true,
        WindowControls: true,
        AppWorkspace: {
          setup: () => {
            started()
            return {}
          },
          template: '<div>Workspace</div>',
        },
      },
    },
  })
  expect(started).not.toHaveBeenCalled()
  expect(wrapper.get('[role="alert"]').text()).toBe('Temporary read failure')
  // Defense in depth: even a session owner created while loading cannot save its defaults.
  const scope = effectScope()
  scope.run(() =>
    persistSession('local', () => ({
      setup: STANDARD_SETUP,
      moves: [],
      clock: null,
      times: null,
      result: null,
    })),
  )
  await flushSavedDocuments()
  scope.stop()
  expect((await read()).sessions.local?.moves).toEqual(['e2e4'])
  api.library = read
  wrapper.getComponent(StartupScreen).vm.$emit('retry')
  await flushPromises()
  expect(started).toHaveBeenCalledOnce()
  expect(useLocalGameStore().moves).toEqual(['e2e4'])
  await flushSavedDocuments()
  expect((await read()).sessions.local?.moves).toEqual(['e2e4'])
  wrapper.unmount()
})

it('blocks startup after an import failure and retries without losing the recovery copy', async () => {
  const { api } = desktop()
  const importLibrary = api.importLibrary
  const saved = JSON.stringify({ version: 1, pgn: '1. e4 *', path: '', orientation: 'white' })
  localStorage.setItem('kchess:analysis:v1', saved)
  api.importLibrary = async () => {
    throw new Error('Disk full')
  }
  await loadLibrary()
  expect(libraryReady.value).toBe(false)
  expect(localStorage.getItem('kchess:analysis:v1')).toBe(saved)
  api.importLibrary = importLibrary
  await loadLibrary()
  expect(libraryReady.value).toBe(true)
  expect(libraryError.value).toBe('')
  expect(localStorage.getItem('kchess:analysis:v1')).toBeNull()
})
