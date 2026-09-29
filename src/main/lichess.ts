import { createHash, randomBytes } from 'node:crypto'
import { createServer } from 'node:http'
import { shell } from 'electron'
import type { AppData, LichessGame, LichessProfile, OnlineEvent } from '../shared/types'
import { addAccount, getToken, loadData, saveGames, saveToken } from './store'

const BASE = 'https://lichess.org'

async function api(path: string, options: RequestInit = {}, token?: string): Promise<Response> {
  const response = await fetch(`${BASE}${path}`, {
    ...options,
    headers: { Accept: 'application/json', ...(token ? { Authorization: `Bearer ${token}` } : {}), ...options.headers }
  })
  if (!response.ok) throw new Error(`Lichess ${response.status}: ${(await response.text()).slice(0, 250)}`)
  return response
}

export async function profile(username: string): Promise<LichessProfile> {
  return (await api(`/api/user/${encodeURIComponent(username)}`)).json() as Promise<LichessProfile>
}

export async function ratingHistory(username: string): Promise<Array<{ name: string; points: Array<[number, number, number, number]> }>> {
  return (await api(`/api/user/${encodeURIComponent(username)}/rating-history`)).json()
}

function normalizeGame(raw: Record<string, any>, account: string): LichessGame {
  const white = String(raw.players?.white?.user?.name ?? raw.players?.white?.user?.id ?? 'Anonymous')
  const black = String(raw.players?.black?.user?.name ?? raw.players?.black?.user?.id ?? 'Anonymous')
  const color: 'white' | 'black' = white.toLowerCase() === account.toLowerCase() ? 'white' : 'black'
  const player = raw.players?.[color]
  const opponent = raw.players?.[color === 'white' ? 'black' : 'white']
  return {
    id: String(raw.id), account, createdAt: Number(raw.createdAt ?? 0), lastMoveAt: Number(raw.lastMoveAt ?? 0),
    rated: Boolean(raw.rated), speed: String(raw.speed ?? ''), perf: String(raw.perf ?? ''), status: String(raw.status ?? ''),
    winner: raw.winner, color, opponent: color === 'white' ? black : white,
    opponentRating: opponent?.rating, playerRating: player?.rating, ratingDiff: player?.ratingDiff,
    opening: raw.opening?.name, moves: String(raw.moves ?? '')
  }
}

export async function syncGames(username?: string): Promise<AppData> {
  const accounts = (await loadData()).accounts.filter(a => !username || a.username.toLowerCase() === username.toLowerCase())
  let result = await loadData()
  for (const account of accounts) {
    const response = await api(`/api/games/user/${encodeURIComponent(account.username)}?max=1000&opening=true`, { headers: { Accept: 'application/x-ndjson' } })
    const text = await response.text()
    const games = text.split('\n').filter(Boolean).map(line => normalizeGame(JSON.parse(line), account.username))
    result = await saveGames(account.username, games)
  }
  return result
}

export async function connectLichess(): Promise<AppData> {
  const verifier = randomBytes(32).toString('base64url')
  const challenge = createHash('sha256').update(verifier).digest('base64url')
  const state = randomBytes(24).toString('base64url')
  const code = await new Promise<{ code: string; redirect: string }>((resolve, reject) => {
    const server = createServer((request, response) => {
      const callback = new URL(request.url ?? '/', `http://127.0.0.1`)
      response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' })
      response.end('<h2>KChess connected to Lichess</h2><p>You can return to the app.</p>')
      server.close()
      clearTimeout(timeout)
      if (callback.searchParams.get('state') !== state || !callback.searchParams.get('code')) reject(new Error('Lichess login was not completed.'))
      else resolve({ code: callback.searchParams.get('code')!, redirect })
    })
    let redirect = ''
    const timeout = setTimeout(() => { server.close(); reject(new Error('Lichess login timed out.')) }, 300_000)
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      if (!address || typeof address === 'string') { reject(new Error('Could not open OAuth callback.')); return }
      redirect = `http://127.0.0.1:${address.port}/callback`
      const url = new URL(`${BASE}/oauth`)
      for (const [key, value] of Object.entries({ response_type: 'code', client_id: 'kchess-desktop', redirect_uri: redirect, scope: 'board:play challenge:write', code_challenge_method: 'S256', code_challenge: challenge, state })) url.searchParams.set(key, value)
      void shell.openExternal(url.toString())
    })
  })
  const body = new URLSearchParams({ grant_type: 'authorization_code', code: code.code, redirect_uri: code.redirect, client_id: 'kchess-desktop', code_verifier: verifier })
  const tokenResponse = await api('/api/token', { method: 'POST', body })
  const token = (await tokenResponse.json() as { access_token: string }).access_token
  const account = await (await api('/api/account', {}, token)).json() as { username: string }
  await saveToken(account.username, token)
  return addAccount(account.username, true)
}

export class OnlineSession {
  private controller: AbortController | null = null
  private gameController: AbortController | null = null
  private currentAccount = ''
  private pendingId = ''
  constructor(private emit: (event: OnlineEvent) => void, private error: (error: string) => void) {}

  async resume(): Promise<string | null> {
    const account = (await loadData()).accounts.find(a => a.connected)
    if (!account) return null
    const token = await getToken(account.username)
    if (!token) return null
    const response = await api('/api/account/playing', {}, token)
    const playing = await response.json() as { nowPlaying?: Array<{ gameId: string }> }
    const id = playing.nowPlaying?.[0]?.gameId
    if (!id) return null
    this.currentAccount = account.username
    this.controller = new AbortController()
    void this.readStream('/api/stream/event', token, this.controller.signal, event => this.emit(event))
    void this.openGame(id, token)
    return id
  }

  async start(options: { minutes: number; increment: number; color: string; target?: string }): Promise<{ id?: string; url?: string; seeking?: boolean }> {
    this.cancel()
    const account = (await loadData()).accounts.find(a => a.connected)
    if (!account) throw new Error('Connect your Lichess account in Settings first.')
    this.currentAccount = account.username
    const token = await getToken(account.username)
    if (!token) throw new Error('Your Lichess login is unavailable. Reconnect in Settings.')
    this.controller = new AbortController()
    void this.readStream('/api/stream/event', token, this.controller.signal, event => {
      if (event.type === 'gameStart' && event.game?.id) { this.pendingId = ''; void this.openGame(event.game.id, token) }
      this.emit(event)
    })
    if (options.target?.trim()) {
      const body = new URLSearchParams({ 'clock.limit': String(options.minutes * 60), 'clock.increment': String(options.increment), color: options.color })
      const response = await api(`/api/challenge/${encodeURIComponent(options.target.trim())}`, { method: 'POST', body }, token)
      const challenge = await response.json() as { id: string; url: string }
      this.pendingId = challenge.id
      return challenge
    }
    const body = new URLSearchParams({ time: String(options.minutes), increment: String(options.increment), color: options.color, rated: 'false' })
    const seekController = new AbortController()
    this.gameController = seekController
    void this.readStream('/api/board/seek', token, seekController.signal, event => this.emit(event), { method: 'POST', body })
    return { seeking: true }
  }

  private async openGame(id: string, token: string): Promise<void> {
    this.gameController?.abort()
    this.gameController = new AbortController()
    await this.readStream(`/api/board/game/stream/${encodeURIComponent(id)}`, token, this.gameController.signal, event => this.emit({ ...event, id }))
  }

  private async readStream(path: string, token: string, signal: AbortSignal, emit: (event: OnlineEvent) => void, options: RequestInit = {}): Promise<void> {
    try {
      const response = await api(path, { ...options, signal, headers: { Accept: 'application/x-ndjson', ...options.headers } }, token)
      const reader = response.body?.getReader()
      if (!reader) throw new Error('Lichess stream unavailable.')
      const decoder = new TextDecoder()
      let buffer = ''
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })
        let end = buffer.indexOf('\n')
        while (end >= 0) {
          const line = buffer.slice(0, end).trim()
          buffer = buffer.slice(end + 1)
          if (line) emit(JSON.parse(line) as OnlineEvent)
          end = buffer.indexOf('\n')
        }
      }
    } catch (error) { if (!signal.aborted) this.error(String(error)) }
  }

  async move(id: string, uci: string): Promise<void> {
    const token = await getToken(this.currentAccount)
    if (!token) throw new Error('Lichess login unavailable.')
    await api(`/api/board/game/${encodeURIComponent(id)}/move/${encodeURIComponent(uci)}`, { method: 'POST' }, token)
  }

  async action(id: string, action: 'resign' | 'abort' | 'takeback' | 'declineTakeback'): Promise<void> {
    const token = await getToken(this.currentAccount)
    if (!token) throw new Error('Lichess login unavailable.')
    const endpoint = { resign: 'resign', abort: 'abort', takeback: 'takeback/yes', declineTakeback: 'takeback/no' }[action]
    await api(`/api/board/game/${encodeURIComponent(id)}/${endpoint}`, { method: 'POST' }, token)
  }

  cancel(): void {
    this.controller?.abort(); this.gameController?.abort()
    this.controller = null; this.gameController = null
    if (this.pendingId) {
      const id = this.pendingId; this.pendingId = ''
      void getToken(this.currentAccount).then(token => {
        if (token) return api(`/api/challenge/${encodeURIComponent(id)}/cancel`, { method: 'POST' }, token).catch(() => undefined)
        return undefined
      })
    }
  }
}
