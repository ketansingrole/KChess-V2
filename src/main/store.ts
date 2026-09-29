import { app, safeStorage } from 'electron'
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { homedir } from 'node:os'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'
import type { AppData, LichessGame, Settings } from '../shared/types'

const defaults: Settings = {
  appearance: 'system', boardPreset: 'lichess', lightSquare: '#f0d9b5',
  darkSquare: '#b58863', soundEnabled: true, soundVolume: 0.7, enginePath: ''
}

const dataPath = (): string => join(app.getPath('userData'), 'kchess-data.json')
const tokenPath = (): string => join(app.getPath('userData'), 'lichess-tokens.json')
const execFileAsync = promisify(execFile)

async function legacyRows<T>(db: string, query: string): Promise<T[]> {
  try {
    const { stdout } = await execFileAsync('/usr/bin/sqlite3', ['-json', db, query], { maxBuffer: 24 * 1024 * 1024 })
    return stdout.trim() ? JSON.parse(stdout) as T[] : []
  } catch { return [] }
}

async function migrateLegacy(): Promise<AppData | null> {
  if (process.platform !== 'darwin') return null
  const db = join(homedir(), '.kchess', 'kchess.db')
  try { await readFile(db) } catch { return null }
  const values = await legacyRows<{ key: string; value: string }>(db, 'SELECT key,value FROM app_settings')
  const get = (key: string): string | undefined => values.find(row => row.key === key)?.value
  const appearance = get('app.appearance_mode')
  const preset = get('board.theme_preset')
  const settings: Settings = {
    ...defaults,
    appearance: appearance === 'light' || appearance === 'dark' ? appearance : 'system',
    boardPreset: preset === 'chess.com' || preset === 'custom' ? preset : 'lichess',
    lightSquare: get('board.light_square_hex') ?? defaults.lightSquare,
    darkSquare: get('board.dark_square_hex') ?? defaults.darkSquare,
    soundEnabled: get('sound.enabled') !== 'false',
    soundVolume: Math.min(1, Number(get('sound.volume') ?? 70) / 100),
    enginePath: get('engine.custom_path') || get('engine.managed_path') || ''
  }
  const oldAccounts = await legacyRows<{ username: string; auth_kind: string; last_synced_at: number | null }>(db, 'SELECT username,auth_kind,last_synced_at FROM lichess_accounts')
  const oldGames = await legacyRows<Record<string, unknown>>(db, 'SELECT game_id,account_username,played_at,rated,speed,perf,status,winner,color,opponent_name,opponent_rating,player_rating,rating_diff,opening_name,moves FROM lichess_games ORDER BY played_at DESC LIMIT 5000')
  const data: AppData = {
    settings,
    accounts: oldAccounts.map(a => ({ username: a.username, connected: a.auth_kind === 'oauth', lastSyncedAt: a.last_synced_at ? a.last_synced_at * 1000 : undefined })),
    games: oldGames.map(g => ({
      id: String(g.game_id), account: String(g.account_username), createdAt: Number(g.played_at), lastMoveAt: Number(g.played_at),
      rated: Boolean(g.rated), speed: String(g.speed), perf: String(g.perf), status: String(g.status),
      winner: g.winner as 'white' | 'black' | undefined, color: g.color as 'white' | 'black', opponent: String(g.opponent_name),
      opponentRating: g.opponent_rating == null ? undefined : Number(g.opponent_rating),
      playerRating: g.player_rating == null ? undefined : Number(g.player_rating),
      ratingDiff: g.rating_diff == null ? undefined : Number(g.rating_diff),
      opening: g.opening_name == null ? undefined : String(g.opening_name), moves: String(g.moves)
    }))
  }
  const oldTokens = await legacyRows<{ username: string; token: string }>(db, 'SELECT username,token FROM lichess_tokens')
  if (safeStorage.isEncryptionAvailable()) for (const row of oldTokens) await saveToken(row.username, row.token)
  return writeData(data)
}

export async function loadData(): Promise<AppData> {
  try {
    const raw = JSON.parse(await readFile(dataPath(), 'utf8')) as Partial<AppData>
    const settings = { ...defaults, ...raw.settings }
    if (settings.soundVolume > 1) settings.soundVolume = Math.min(1, settings.soundVolume / 100)
    const accounts = (raw.accounts ?? []).map(account => ({ ...account, lastSyncedAt: account.lastSyncedAt && account.lastSyncedAt < 1e11 ? account.lastSyncedAt * 1000 : account.lastSyncedAt }))
    return { settings, accounts, games: raw.games ?? [] }
  } catch {
    return await migrateLegacy() ?? { settings: { ...defaults }, accounts: [], games: [] }
  }
}

async function writeData(data: AppData): Promise<AppData> {
  await mkdir(dirname(dataPath()), { recursive: true })
  await writeFile(dataPath(), JSON.stringify(data, null, 2), { mode: 0o600 })
  return data
}

export async function saveSettings(settings: Settings): Promise<Settings> {
  const data = await loadData()
  data.settings = settings
  await writeData(data)
  return settings
}

export async function addAccount(username: string, connected = false): Promise<AppData> {
  const normalized = username.trim()
  if (!/^[a-zA-Z0-9_-]{2,30}$/.test(normalized)) throw new Error('Enter a valid Lichess username.')
  const data = await loadData()
  const account = data.accounts.find(a => a.username.toLowerCase() === normalized.toLowerCase())
  if (account) account.connected ||= connected
  else data.accounts.push({ username: normalized, connected })
  return writeData(data)
}

export async function removeAccount(username: string): Promise<AppData> {
  const data = await loadData()
  data.accounts = data.accounts.filter(a => a.username.toLowerCase() !== username.toLowerCase())
  data.games = data.games.filter(g => g.account.toLowerCase() !== username.toLowerCase())
  await removeToken(username)
  return writeData(data)
}

export async function saveGames(username: string, games: LichessGame[]): Promise<AppData> {
  const data = await loadData()
  const existing = new Map(data.games.map(game => [`${game.account.toLowerCase()}:${game.id}`, game]))
  for (const game of games) existing.set(`${game.account.toLowerCase()}:${game.id}`, game)
  data.games = [...existing.values()].sort((a, b) => b.createdAt - a.createdAt).slice(0, 5000)
  const account = data.accounts.find(a => a.username.toLowerCase() === username.toLowerCase())
  if (account) account.lastSyncedAt = Date.now()
  return writeData(data)
}

async function readTokens(): Promise<Record<string, string>> {
  try { return JSON.parse(await readFile(tokenPath(), 'utf8')) as Record<string, string> }
  catch { return {} }
}

export async function saveToken(username: string, token: string): Promise<void> {
  if (!safeStorage.isEncryptionAvailable()) throw new Error('OS credential encryption is unavailable.')
  const tokens = await readTokens()
  tokens[username.toLowerCase()] = safeStorage.encryptString(token).toString('base64')
  await mkdir(dirname(tokenPath()), { recursive: true })
  await writeFile(tokenPath(), JSON.stringify(tokens), { mode: 0o600 })
}

export async function getToken(username: string): Promise<string | null> {
  const encoded = (await readTokens())[username.toLowerCase()]
  if (!encoded || !safeStorage.isEncryptionAvailable()) return null
  try { return safeStorage.decryptString(Buffer.from(encoded, 'base64')) }
  catch { return null }
}

async function removeToken(username: string): Promise<void> {
  const tokens = await readTokens()
  delete tokens[username.toLowerCase()]
  await writeFile(tokenPath(), JSON.stringify(tokens), { mode: 0o600 })
}
