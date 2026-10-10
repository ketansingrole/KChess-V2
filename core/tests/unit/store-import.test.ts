import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'
import { closeNativeCore, nativeCallSync } from '../../src/services/nativeCore'
import { nativeRules } from '../../src/services/native'
import { setPlatform } from '../../src/services/platform'
import { getToken, getSettings, loadData, gamePage } from '../../../tests/fixtures/nativeGames'
import { fakeSecrets, testPlatform } from '../../../tests/fixtures/corePlatform'

/**
 * The one-time import of an empty profile (`crates/kchess-core/src/store/import.rs`, driven by
 * `ensureMigrated` in `store.ts`): the JSON backup with its token file, and the earlier main
 * database, whose plaintext tokens are encrypted here with the platform's secret store.
 */

const roots: string[] = []
afterEach(async () => {
  await closeNativeCore()
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true })
})

function directory(): string {
  const created = mkdtempSync(join(tmpdir(), 'kchess-import-'))
  roots.push(created)
  return created
}

/** Create a database file as the core does, then write rows into it (an earlier release's database). */
async function earlierDatabase(path: string, sql: string): Promise<void> {
  const native = nativeRules()
  if (!native) throw new Error('The native rules are not built (pnpm run build:native).')
  const core = new native.NativeCore(
    { dataDir: path },
    () => {},
    () => {},
  )
  try {
    core.callSync('store.open', '[]')
    core.callSync('store.debug.exec', JSON.stringify([sql, []]))
  } finally {
    await core.close()
  }
}

describe('one-time import into an empty profile', () => {
  it('takes the JSON backup, its tokens, and archives both files', async () => {
    const dataDir = directory()
    writeFileSync(
      join(dataDir, 'kchess-data.json'),
      JSON.stringify({
        settings: {
          appearance: 'dark',
          boardPreset: 'chess.com',
          soundVolume: 80,
          promotion: 'odd',
        },
        accounts: [
          { username: 'Alice', connected: true },
          { username: 'Bob', connected: false },
        ],
        games: [
          {
            id: 'Old0001',
            account: 'Alice',
            createdAt: 1,
            lastMoveAt: 1,
            rated: true,
            speed: 'blitz',
            perf: 'blitz',
            status: 'mate',
            color: 'white',
            opponent: 'Bob',
            moves: 'e4 e5',
          },
          {
            id: 'Live0001',
            account: 'Alice',
            createdAt: 2,
            lastMoveAt: 2,
            rated: false,
            speed: 'rapid',
            perf: 'rapid',
            status: 'started',
            color: 'black',
            opponent: 'Bob',
            moves: '',
          },
        ],
      }),
    )
    writeFileSync(
      join(dataDir, 'lichess-tokens.json'),
      JSON.stringify({ Alice: Buffer.from('lip_alice').toString('base64') }),
    )
    setPlatform(testPlatform({ dataDir, secrets: fakeSecrets(true) }))

    const settings = await getSettings()
    expect(settings.appearance).toBe('dark')
    expect(settings.boardTheme).toBe('green')
    expect(settings.promotion).toBe('ask')
    expect(settings.soundVolume).toBe(0.8)

    const data = await loadData()
    expect(data.accounts.map((account) => account.username)).toEqual(['Alice', 'Bob'])
    expect(data.accounts[0]?.connected).toBe(true)
    expect(data.gameCount).toBe(1)
    expect((await gamePage({ offset: 0, limit: 10 })).games.map((game) => game.id)).toEqual([
      'Old0001',
    ])
    expect(await getToken('Alice')).toBe('lip_alice')

    expect(existsSync(join(dataDir, 'kchess-data.json'))).toBe(false)
    expect(existsSync(join(dataDir, 'kchess-data.json.bak'))).toBe(true)
    expect(existsSync(join(dataDir, 'lichess-tokens.json.bak'))).toBe(true)
  })

  it('imports an earlier main database once, encrypting its plaintext tokens', async () => {
    const earlier = directory()
    const legacyPath = join(earlier, 'kchess.db')
    await earlierDatabase(
      earlier,
      `CREATE TABLE app_settings (key TEXT, value TEXT);
       INSERT INTO app_settings VALUES ('app.appearance_mode', 'light'), ('sound.enabled', 'false'),
         ('sound.volume', '40'), ('engine.custom_path', '/opt/fish');
       CREATE TABLE lichess_accounts (username TEXT, auth_kind TEXT, last_synced_at INTEGER);
       INSERT INTO lichess_accounts VALUES ('Carol', 'oauth', 5);
       CREATE TABLE lichess_games (game_id TEXT, account_username TEXT, played_at INTEGER, rated INTEGER,
         speed TEXT, perf TEXT, status TEXT, winner TEXT, color TEXT, opponent_name TEXT,
         opponent_rating INTEGER, player_rating INTEGER, rating_diff INTEGER, opening_name TEXT, moves TEXT);
       INSERT INTO lichess_games VALUES ('Legacy01', 'Carol', 7, 1, 'bullet', 'bullet', 'resign', 'black',
         'white', 'Dan', 1500, 1510, -4, NULL, 'e4 e5');
       CREATE TABLE lichess_tokens (username TEXT, token TEXT);
       INSERT INTO lichess_tokens VALUES ('Carol', 'lip_carol');`,
    )
    const dataDir = directory()
    setPlatform(
      testPlatform({ dataDir, legacyDatabasePath: legacyPath, secrets: fakeSecrets(true) }),
    )

    const settings = await getSettings()
    expect(settings.appearance).toBe('light')
    expect(settings.soundEnabled).toBe(false)
    expect(settings.soundVolume).toBe(0.4)
    expect(settings.enginePath).toBe('/opt/fish')
    expect((await loadData()).accounts).toEqual([
      { username: 'Carol', connected: true, lastSyncedAt: undefined },
    ])
    expect((await gamePage({ offset: 0, limit: 10 })).games[0]).toMatchObject({
      id: 'Legacy01',
      opponentRating: 1500,
    })
    expect(await getToken('Carol')).toBe('lip_carol')

    // Once imported, the profile is never imported again: a later change to the earlier database
    // does not reach it.
    await closeNativeCore()
    setPlatform(
      testPlatform({ dataDir, legacyDatabasePath: legacyPath, secrets: fakeSecrets(true) }),
    )
    expect((await loadData()).gameCount).toBe(1)
    expect(nativeCallSync('store.setupPositionLookup.explorerAccount')).toBe('Carol')
  })

  it('leaves tokens out of an earlier database when OS encryption is unavailable', async () => {
    const earlier = directory()
    const legacyPath = join(earlier, 'kchess.db')
    await earlierDatabase(
      earlier,
      `CREATE TABLE lichess_accounts (username TEXT, auth_kind TEXT, last_synced_at INTEGER);
       INSERT INTO lichess_accounts VALUES ('Erin', 'password', NULL);
       CREATE TABLE lichess_tokens (username TEXT, token TEXT);
       INSERT INTO lichess_tokens VALUES ('Erin', 'lip_erin');`,
    )
    const dataDir = directory()
    setPlatform(
      testPlatform({ dataDir, legacyDatabasePath: legacyPath, secrets: fakeSecrets(false) }),
    )

    expect((await loadData()).accounts).toEqual([
      { username: 'Erin', connected: false, lastSyncedAt: undefined },
    ])
    expect(await getToken('Erin')).toBeNull()
  })

  it('reads damaged backups as no backup and imports nothing from them', async () => {
    const dataDir = directory()
    writeFileSync(join(dataDir, 'kchess-data.json'), '{ not json')
    writeFileSync(join(dataDir, 'lichess-tokens.json'), '{ not json')
    setPlatform(testPlatform({ dataDir, secrets: fakeSecrets(true) }))

    expect((await loadData()).accounts).toEqual([])
    expect(readFileSync(join(dataDir, 'kchess-data.json'), 'utf8')).toBe('{ not json')
  })
})

describe('legacy NULLs', () => {
  it('rolls the whole earlier import back when a token is NULL and encryption is available', async () => {
    const earlier = directory()
    const legacyPath = join(earlier, 'kchess.db')
    await earlierDatabase(
      earlier,
      `CREATE TABLE app_settings (key TEXT, value TEXT);
       INSERT INTO app_settings VALUES ('app.appearance_mode', 'dark');
       CREATE TABLE lichess_accounts (username TEXT, auth_kind TEXT, last_synced_at INTEGER);
       INSERT INTO lichess_accounts VALUES ('Frank', 'oauth', NULL);
       CREATE TABLE lichess_tokens (username TEXT, token TEXT);
       INSERT INTO lichess_tokens VALUES ('Frank', NULL);`,
    )
    const dataDir = directory()
    setPlatform(
      testPlatform({ dataDir, legacyDatabasePath: legacyPath, secrets: fakeSecrets(true) }),
    )

    expect((await getSettings()).appearance).toBe('system')
    expect((await loadData()).accounts).toEqual([])
  })

  it('keeps NULL as NULL, never the text "null", and skips NULL tokens without encryption', async () => {
    const earlier = directory()
    const legacyPath = join(earlier, 'kchess.db')
    await earlierDatabase(
      earlier,
      `CREATE TABLE app_settings (key TEXT, value TEXT);
       INSERT INTO app_settings VALUES ('app.appearance_mode', 'dark');
       CREATE TABLE lichess_accounts (username TEXT, auth_kind TEXT, last_synced_at INTEGER);
       INSERT INTO lichess_accounts VALUES (NULL, 'password', NULL);
       CREATE TABLE lichess_tokens (username TEXT, token TEXT);
       INSERT INTO lichess_tokens VALUES (NULL, 'lip_orphan'), ('Grace', NULL);`,
    )
    const dataDir = directory()
    setPlatform(
      testPlatform({ dataDir, legacyDatabasePath: legacyPath, secrets: fakeSecrets(false) }),
    )

    expect((await getSettings()).appearance).toBe('dark')
    expect(nativeCallSync('store.debug.query', 'SELECT username FROM accounts', [])).toEqual([
      { username: null },
    ])
    expect(
      nativeCallSync(
        'store.debug.query',
        "SELECT COUNT(*) AS n FROM tokens WHERE username = 'null'",
        [],
      ),
    ).toEqual([{ n: 0 }])
  })
})
