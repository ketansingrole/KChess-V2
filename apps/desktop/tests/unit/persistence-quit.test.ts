import { expect, it, vi } from 'vitest'
import { effectScope } from 'vue'
import { desktop } from './fixtures'
import { useLocalGameStore } from '../../app/stores/local'
import { useKChessStore } from '../../app/stores/kchess'
import {
  flushSavedDocuments,
  persistSession,
  registerPersistenceFlush,
} from '../../app/utils/library'
import type { Settings } from '@kchess/core/contracts/types'

it('flushes the latest game, archive and identity before their debounce fires', async () => {
  desktop()
  const game = useLocalGameStore()
  game.start()
  expect(game.move('e2e4')).toBe(true)
  await flushSavedDocuments()
  const library = await window.kchess.library()
  expect(library.sessions.local?.moves).toEqual(['e2e4'])
  expect(library.games).toHaveLength(1)
  expect(library.sessions['archive:board']?.id).toBe(library.games[0]?.id)
})
it('flushes a pending settings change without waiting for its timer', async () => {
  const save = vi.fn(async (settings: Settings) => settings)
  desktop({ saveSettings: save })
  const store = useKChessStore()
  await store.init()
  store.settings!.appearance = 'dark'
  await flushSavedDocuments()
  expect(save).toHaveBeenCalledWith(expect.objectContaining({ appearance: 'dark' }))
})
it('detaches persistence callbacks when their scope is disposed', async () => {
  const saveSession = vi.fn(async () => {})
  desktop({ saveSession })
  const scope = effectScope()
  scope.run(() =>
    persistSession('analysis', () => ({
      pgn: '*',
      path: '',
      orientation: 'white',
      study: '',
      chapter: '',
    })),
  )
  scope.stop()
  const count = saveSession.mock.calls.length
  await flushSavedDocuments()
  expect(saveSession).toHaveBeenCalledTimes(count)
})
it('flushes remaining owners even if one save rejects', async () => {
  const failed = registerPersistenceFlush(async () => {
    throw new Error('Failed save')
  })
  const done = vi.fn(async () => {})
  const unregister = registerPersistenceFlush(done)
  await flushSavedDocuments()
  expect(done).toHaveBeenCalledOnce()
  failed()
  unregister()
})
