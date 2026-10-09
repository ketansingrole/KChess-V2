import { flushPromises } from '@vue/test-utils'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { useKChessStore } from '../../app/stores/kchess'
import { usePuzzleStore } from '../../app/stores/puzzles'
import { DEFAULT_SETTINGS } from '@kchess/core/contracts/defaultSettings'
import { desktop, puzzle } from './fixtures'

afterEach(() => vi.restoreAllMocks())

async function start(connected: boolean, online: boolean) {
  vi.spyOn(navigator, 'onLine', 'get').mockReturnValue(online)
  const next = vi.fn(async () => ({ puzzle }))
  const local = vi.fn(async () => [puzzle])
  const solve = vi.fn(async () => ({}))
  desktop({
    loadData: async () => ({
      settings: { ...DEFAULT_SETTINGS },
      accounts: connected ? [{ username: 'Alice', connected: true }] : [],
      gameCount: 0,
    }),
    puzzleNext: next,
    localPuzzles: local,
    puzzleSolve: solve,
  })
  const app = useKChessStore()
  await app.init()
  return { app, puzzles: usePuzzleStore(), next, local, solve }
}

describe('account-free start', () => {
  it('keeps followed public players out of the signed-in Home selection', async () => {
    const profile = vi.fn(async () => ({
      id: 'publicplayer',
      username: 'PublicPlayer',
      url: 'https://lichess.org/@/PublicPlayer',
    }))
    desktop({
      loadData: async () => ({
        settings: { ...DEFAULT_SETTINGS },
        accounts: [{ username: 'PublicPlayer', connected: false }],
        gameCount: 197,
      }),
      profile,
    })
    const app = useKChessStore()
    await app.init()
    await flushPromises()
    expect(app.selectedAccount).toBe('')
    expect(app.activeOnlineAccount).toBe('')
    expect(app.profile).toBeNull()
    expect(app.recentGames).toEqual([])
    expect(app.trackedAccounts).toHaveLength(1)
    expect(profile).not.toHaveBeenCalled()
  })

  it('selects an owned account when a followed player comes first in the stored list', async () => {
    desktop({
      loadData: async () => ({
        settings: { ...DEFAULT_SETTINGS },
        accounts: [
          { username: 'PublicPlayer', connected: false },
          { username: 'Alice', connected: true },
        ],
        gameCount: 197,
      }),
    })
    const app = useKChessStore()
    await app.init()
    expect(app.selectedAccount).toBe('Alice')
    expect(app.connectedAccounts.map((a) => a.username)).toEqual(['Alice'])
  })

  it.each([false, true])(
    'starts online in practice with connected account = %s',
    async (connected) => {
      const { puzzles, next, solve, local } = await start(connected, true)
      expect(puzzles.mode).toBe('practice')
      await puzzles.loadNext()
      expect(puzzles.phase).toBe('ready')
      expect(next).toHaveBeenCalledOnce()
      expect(local).not.toHaveBeenCalled()
      await puzzles.report(true)
      expect(solve).not.toHaveBeenCalled()
    },
  )

  it.each([false, true])(
    'starts offline with local puzzles and connected account = %s',
    async (connected) => {
      const { app, puzzles, next, local } = await start(connected, false)
      expect(app.ready).toBe(true)
      expect(app.activeOnlineAccount).toBe(connected ? 'Alice' : '')
      expect(puzzles.mode).toBe('offline')
      await puzzles.loadNext()
      expect(puzzles.phase).toBe('ready')
      expect(local).toHaveBeenCalledOnce()
      expect(next).not.toHaveBeenCalled()
    },
  )

  it('keeps an explicit rated choice and requires an account before fetching', async () => {
    localStorage.setItem('kchess:puzzle-mode', 'rated')
    const { puzzles, next } = await start(false, true)
    expect(puzzles.mode).toBe('rated')
    await puzzles.loadNext()
    expect(puzzles.phase).toBe('noaccount')
    expect(next).not.toHaveBeenCalled()
  })

  it('offers downloaded training after losing connectivity without discarding the mode', async () => {
    const { puzzles, next, local } = await start(false, true)
    window.dispatchEvent(new Event('offline'))
    await puzzles.loadNext()
    expect(puzzles.mode).toBe('practice')
    expect(puzzles.phase).toBe('error')
    expect(puzzles.trainError).toContain('Choose Downloaded')
    expect(next).not.toHaveBeenCalled()
    puzzles.mode = 'offline'
    await puzzles.loadNext()
    expect(puzzles.phase).toBe('ready')
    expect(local).toHaveBeenCalledOnce()
  })
})
