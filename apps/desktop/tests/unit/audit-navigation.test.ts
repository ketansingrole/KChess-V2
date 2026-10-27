import { expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { useKChessStore } from '../../app/stores/kchess'
import { desktop } from './fixtures'
import type { AppData, LichessGame } from '@kchess/contracts/types'

const accounts: AppData['accounts'] = [
  { username: 'Alice', connected: true },
  { username: 'Carol', connected: false },
  { username: 'Dave', connected: false },
]

async function setup(overrides: Parameters<typeof desktop>[0] = {}) {
  const fixture = desktop(overrides)
  const store = useKChessStore()
  await store.init()
  await flushPromises()
  vi.mocked(navigateTo).mockClear()
  return { store, ...fixture }
}

// A recent Lichess game opened History on its Computer tab, where the game never showed.
it('opens a Lichess game on History’s Lichess tab', async () => {
  const { store } = await setup({ gamePgn: async () => '1. e4 *' })
  await store.openReview({ id: 'g1', account: 'Alice' } as LichessGame)
  expect(navigateTo).toHaveBeenCalledWith({ path: '/history', query: { source: 'lichess' } })
  expect(store.reviewGame?.id).toBe('g1')
})

// "Set up Stockfish" left Settings on whatever section was open last.
it('opens Settings at the section a link names', async () => {
  const { store } = await setup()
  store.openSettings('engine')
  expect(store.settingsSection).toBe('engine')
  expect(navigateTo).toHaveBeenCalledWith('/settings')
})

// Following a name already listed synced whoever happened to be the newest row.
it('syncs the player just followed, not the newest row', async () => {
  const reply = async () => ({ ...store.data!, accounts, gameCount: 0 }) as AppData
  const syncGames = vi.fn(reply)
  const { store } = await setup({
    addAccount: reply,
    usage: async () => ({ accounts: {} }) as never,
    syncGames,
  })
  store.usernameInput = '@carol'
  await store.addAccount()
  expect(syncGames).toHaveBeenCalledWith('Carol')
})
