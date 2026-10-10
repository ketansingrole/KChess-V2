import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeEach, describe, expect, it } from 'vitest'
import { DEFAULT_SETTINGS } from '@kchess/contracts/defaultSettings'
import type { LichessGame } from '@kchess/contracts/types'
import { closeNativeCore } from '@kchess/native/nativeCore'
import { setPlatform } from '@kchess/native/platform'
import {
  addAccount,
  addFriends,
  clearAccountData,
  dismissedFriends,
  gameLibraryOverview,
  gamePage,
  gamePgn,
  gameRatingHistory,
  getSettings,
  getToken,
  loadData,
  logoutAccounts,
  pendingGameIds,
  readApiCache,
  removeAccount,
  resetStore,
  saveGames,
  saveGamesPage,
  saveLogin,
  saveSettings,
  writeApiCache,
} from '../fixtures/nativeGames'
import { fakeSecrets, testPlatform } from '../fixtures/corePlatform'
import type { SecretStore } from '@kchess/native/platform'

/** The account, token and cache calls of the storage wrappers, through the native core. */

const profiles: string[] = []
let dataDir = ''
let secrets: ReturnType<typeof fakeSecrets>

const game = (id: string, overrides: Partial<LichessGame> = {}): LichessGame => ({
  id,
  account: 'Alice',
  createdAt: 1_000,
  lastMoveAt: 1_000,
  rated: true,
  speed: 'blitz',
  perf: 'blitz',
  status: 'mate',
  winner: 'white',
  color: 'white',
  opponent: 'Bob',
  opponentRating: 1500,
  playerRating: 1520,
  ratingDiff: 12,
  moves: 'e2e4 e7e5',
  ...overrides,
})

beforeEach(async () => {
  await closeNativeCore()
  dataDir = mkdtempSync(join(tmpdir(), 'kchess-accounts-'))
  profiles.push(dataDir)
  secrets = fakeSecrets(true)
  setPlatform(testPlatform({ dataDir, secrets }))
})
afterAll(async () => {
  await closeNativeCore()
  for (const profile of profiles) rmSync(profile, { recursive: true, force: true })
})

describe('logins and tokens', () => {
  it('stores a login with its token, and returns the token decrypted', async () => {
    const data = await saveLogin('Alice', 'lip_alice', () => true)
    expect(data.accounts.map((account) => account.username)).toEqual(['Alice'])
    expect(data.accounts[0]?.connected).toBe(true)
    expect(await getToken('Alice')).toBe('lip_alice')
    expect(await getToken('nobody')).toBeNull()
  })

  it('refuses a login without OS encryption, and one that a logout overtook', async () => {
    secrets.enabled = false
    await expect(saveLogin('Alice', 'lip_alice', () => true)).rejects.toThrow(
      'OS credential encryption is unavailable.',
    )
    secrets.enabled = true
    await expect(saveLogin('Alice', 'lip_alice', () => false)).rejects.toThrow(
      'Login was cancelled by logout.',
    )
    expect((await loadData()).accounts).toEqual([])
  })

  it('reads no token without OS encryption, and none that cannot be decrypted', async () => {
    await saveLogin('Alice', 'lip_alice', () => true)
    secrets.enabled = false
    expect(await getToken('Alice')).toBeNull()
    secrets.enabled = true
    const broken: SecretStore = {
      ...secrets,
      decrypt: () => {
        throw new Error('keychain locked')
      },
    }
    // One live core per profile: the first one closes before the same profile opens again.
    await closeNativeCore()
    setPlatform(testPlatform({ dataDir, secrets: broken }))
    expect(await getToken('Alice')).toBeNull()
    setPlatform(testPlatform({ dataDir, secrets }))
  })
})

describe('accounts', () => {
  it('adds, follows, dismisses and removes accounts', async () => {
    await addAccount('Alice', true)
    await expect(addAccount('no good name!')).rejects.toThrow()
    const friends = await addFriends(['Bob', 'carol'])
    expect(friends.accounts.map((account) => account.username)).toEqual(['Alice', 'Bob', 'carol'])
    expect([...(await dismissedFriends())]).toEqual([])
    await removeAccount('carol')
    expect([...(await dismissedFriends())]).toEqual(['carol'])
    await removeAccount('Bob')
    const after = await loadData()
    expect(after.accounts.map((account) => account.username)).toEqual(['Alice'])
  })

  it("logs out one account or all of them, and clears one account's downloads", async () => {
    await saveLogin('Alice', 'lip_alice', () => true)
    await saveLogin('Carol', 'lip_carol', () => true)
    await saveGames('Alice', [game('Game0001')], 2_000)
    await writeApiCache('alice:profile', { name: 'Alice' })
    await clearAccountData('Alice')
    expect((await loadData()).gameCount).toBe(0)
    expect(await readApiCache('alice:profile')).toBeNull()
    await saveGames('Alice', [game('Game0002')], 2_000)
    expect((await logoutAccounts('Alice')).accounts.map((account) => account.username)).toEqual([
      'Carol',
    ])
    expect(await getToken('Alice')).toBeNull()
    expect((await logoutAccounts()).accounts).toEqual([])
    expect(await getToken('Carol')).toBeNull()
  })
})

describe('played games', () => {
  it('saves games, lists them, reports their overview and rating history, and reads a PGN', async () => {
    await addAccount('Alice', true)
    await saveGames('Alice', [game('Game0001'), game('Game0002', { createdAt: 2_000 })], 3_000)
    expect((await loadData()).gameCount).toBe(2)
    const page = await gamePage({ offset: 0, limit: 1, account: 'alice' })
    expect(page.total).toBe(2)
    expect(page.games[0]?.id).toBe('Game0002')
    await expect(gamePage({ offset: -1, limit: 1 } as never)).rejects.toThrow()
    expect((await gameLibraryOverview()).byAccount.alice).toMatchObject({ total: 2, win: 2 })
    expect(await gameRatingHistory('Alice')).toEqual([expect.objectContaining({ name: 'blitz' })])
    expect(await gamePgn('Alice', 'Game0001')).toBeNull()
    expect(await pendingGameIds('Alice')).toEqual([])
  })

  it('refuses a sync that was cancelled, or whose account was removed meanwhile', async () => {
    await addAccount('Alice', true)
    await expect(saveGamesPage('Alice', [game('Game0001')], [], () => false)).rejects.toThrow(
      'Sync was cancelled.',
    )
    await expect(saveGames('Alice', [], 1, () => false)).rejects.toThrow('Sync was cancelled.')
    await removeAccount('Alice')
    await expect(saveGamesPage('Alice', [game('Game0003')])).rejects.toThrow(
      'This account was removed during sync.',
    )
  })

  it('keeps an in-progress game as pending until it finishes', async () => {
    await addAccount('Alice', true)
    await saveGamesPage('Alice', [game('Live0001', { status: 'started', winner: undefined })])
    expect(await pendingGameIds('Alice')).toEqual(['Live0001'])
    expect((await loadData()).gameCount).toBe(0)
  })
})

describe('settings, cache and reset', () => {
  it('saves settings, reads them back, and keeps cache entries only while not cancelled', async () => {
    const saved = await saveSettings({ ...DEFAULT_SETTINGS, zenMode: true })
    expect(saved.zenMode).toBe(true)
    expect((await getSettings()).zenMode).toBe(true)
    await writeApiCache('ratings', { perf: 1 }, () => false)
    expect(await readApiCache('ratings')).toBeNull()
    await writeApiCache('ratings', { perf: 1 })
    expect(await readApiCache<{ perf: number }>('ratings')).toMatchObject({
      value: { perf: 1 },
    })
    resetStore()
  })
})
