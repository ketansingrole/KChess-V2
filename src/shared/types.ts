export type Appearance = 'system' | 'light' | 'dark'
export type BoardPreset = 'lichess' | 'chess.com' | 'custom'

export interface Settings {
  appearance: Appearance
  boardPreset: BoardPreset
  lightSquare: string
  darkSquare: string
  soundEnabled: boolean
  soundVolume: number
  enginePath: string
}

export interface LichessAccount {
  username: string
  connected: boolean
  lastSyncedAt?: number
}

export interface LichessGame {
  id: string
  account: string
  createdAt: number
  lastMoveAt: number
  rated: boolean
  speed: string
  perf: string
  status: string
  winner?: 'white' | 'black'
  color: 'white' | 'black'
  opponent: string
  opponentRating?: number
  playerRating?: number
  ratingDiff?: number
  opening?: string
  moves: string
}

export interface AppData {
  settings: Settings
  accounts: LichessAccount[]
  games: LichessGame[]
}

export interface LichessProfile {
  username: string
  count: { all: number; win: number; loss: number; draw: number }
  perfs: Record<string, { rating?: number; games?: number; rd?: number; prog?: number }>
}

export interface OnlineEvent {
  type: string
  game?: { id: string; color?: 'white' | 'black'; fen?: string; opponent?: { username?: string } }
  id?: string
  moves?: string
  status?: string
  wtime?: number
  btime?: number
  winner?: string
  white?: { name?: string; id?: string }
  black?: { name?: string; id?: string }
  initialFen?: string
  state?: OnlineEvent
  [key: string]: unknown
}

export interface DesktopApi {
  loadData(): Promise<AppData>
  saveSettings(settings: Settings): Promise<Settings>
  addAccount(username: string): Promise<AppData>
  removeAccount(username: string): Promise<AppData>
  syncGames(username?: string): Promise<AppData>
  profile(username: string): Promise<LichessProfile>
  ratingHistory(username: string): Promise<Array<{ name: string; points: Array<[number, number, number, number]> }>>
  connectLichess(): Promise<AppData>
  engineStatus(path?: string): Promise<{ path: string; ready: boolean }>
  chooseEngine(): Promise<string | null>
  installEngine(): Promise<{ path: string; version: string }>
  bestMove(moves: string[], level: 'low' | 'medium' | 'high', path?: string): Promise<string>
  startOnline(options: { minutes: number; increment: number; color: string; target?: string }): Promise<{ id?: string; url?: string; seeking?: boolean }>
  resumeOnline(): Promise<string | null>
  cancelOnline(id?: string): Promise<void>
  playOnline(id: string, move: string): Promise<void>
  onlineAction(id: string, action: 'resign' | 'abort' | 'takeback' | 'declineTakeback'): Promise<void>
  onOnlineEvent(callback: (event: OnlineEvent) => void): () => void
  onOnlineError(callback: (message: string) => void): () => void
}
