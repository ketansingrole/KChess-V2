<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { Chess, type Square } from 'chess.js'
import ChessBoard from './components/ChessBoard.vue'
import type { AppData, LichessGame, LichessProfile, OnlineEvent, Settings } from '../../shared/types'

type Page = 'dashboard' | 'online' | 'computer' | 'history' | 'settings'
const nav: { id: Page; label: string; icon: string }[] = [
  { id: 'dashboard', label: 'Dashboard', icon: 'i-lucide-layout-dashboard' },
  { id: 'online', label: 'Play Online', icon: 'i-lucide-globe-2' },
  { id: 'computer', label: 'Play with Computer', icon: 'i-lucide-monitor' },
  { id: 'history', label: 'History', icon: 'i-lucide-history' },
  { id: 'settings', label: 'Settings', icon: 'i-lucide-settings-2' }
]
const page = ref<Page>('dashboard')
const searchOpen = ref(false)
const searchQuery = ref('')
const searchIndex = ref(0)
const searchInput = ref<HTMLInputElement | null>(null)
const searchResults = computed(() => nav.filter(item => `${item.label} ${item.id}`.toLowerCase().includes(searchQuery.value.toLowerCase())))
const data = ref<AppData | null>(null)
const settings = ref<Settings | null>(null)
const busy = ref(false)
const message = ref('')
const error = ref('')
const usernameInput = ref('')
const selectedAccount = ref('')
const profile = ref<LichessProfile | null>(null)
const ratingHistories = ref<Array<{ name: string; points: Array<[number, number, number, number]> }>>([])
const chartMode = ref('Blitz')
const chartRange = ref('All')
const historyResult = ref('all')
const historyRated = ref('all')
const historyAccount = ref('all')
const historyPage = ref(0)
const historyPageSize = ref(20)
const reviewGame = ref<LichessGame | null>(null)
const reviewPly = ref(0)
const reviewPlaying = ref(false)
const localMoves = ref<string[]>([])
const localPly = ref(0)
const level = ref<'low' | 'medium' | 'high'>('medium')
const userColor = ref<'white' | 'black'>('white')
const flipped = ref(false)
const thinking = ref(false)
const gameEpoch = ref(0)
const engineReady = ref(false)
const onlinePhase = ref<'idle' | 'seeking' | 'playing' | 'finished'>('idle')
const onlineId = ref('')
const onlineMoves = ref<string[]>([])
const onlineColor = ref<'white' | 'black'>('white')
const onlineOpponent = ref('Opponent')
const onlineStatus = ref('')
const onlineClock = ref({ white: 0, black: 0 })
const onlineMinutes = ref(15)
const onlineIncrement = ref(10)
const onlineTarget = ref('')
const onlineChoice = ref('random')
const onlineStart = ref(Date.now())
let offOnline: (() => void) | undefined
let offOnlineError: (() => void) | undefined
let ticker: ReturnType<typeof setInterval> | undefined

const localGame = computed(() => {
  const chess = new Chess()
  for (const move of localMoves.value) { try { chess.move(move) } catch { break } }
  return chess
})
const localDisplay = computed(() => position(localMoves.value, localPly.value))
const localHistory = computed(() => localGame.value.history({ verbose: true }))
const localLast = computed(() => localPly.value ? localHistory.value[localPly.value - 1] : null)
const onlineGame = computed(() => position(onlineMoves.value, onlineMoves.value.length))
const reviewMoves = computed(() => reviewGame.value?.moves.split(/\s+/).filter(Boolean) ?? [])
const reviewPosition = computed(() => position(reviewMoves.value, reviewPly.value))
const reviewHistory = computed(() => {
  const chess = new Chess()
  const result: string[] = []
  for (const move of reviewMoves.value) { try { result.push(chess.move(move).san) } catch { break } }
  return result
})
const games = computed(() => data.value?.games ?? [])
const filteredGames = computed(() => games.value.filter(game => {
  if (historyAccount.value !== 'all' && game.account !== historyAccount.value) return false
  if (historyRated.value !== 'all' && (game.rated ? 'rated' : 'casual') !== historyRated.value) return false
  return historyResult.value === 'all' || gameResult(game) === historyResult.value
}))
const visibleGames = computed(() => filteredGames.value.slice(historyPage.value * historyPageSize.value, (historyPage.value + 1) * historyPageSize.value))
const recentGames = computed(() => games.value.filter(g => g.account === selectedAccount.value).slice(0, 6))
const counts = computed(() => {
  const own = games.value.filter(g => g.account === selectedAccount.value)
  return { win: own.filter(g => gameResult(g) === 'win').length, loss: own.filter(g => gameResult(g) === 'loss').length, draw: own.filter(g => gameResult(g) === 'draw').length }
})
const chartPoints = computed(() => {
  const cutoffDays: Record<string, number> = { '1M': 30, '3M': 90, '6M': 180, '1Y': 365 }
  const cutoff = chartRange.value === 'YTD' ? new Date(new Date().getFullYear(), 0, 1).getTime() : chartRange.value === 'All' ? 0 : Date.now() - cutoffDays[chartRange.value] * 86_400_000
  const points = ratingHistories.value.find(item => item.name === chartMode.value)?.points ?? []
  const values = points.filter(p => Date.UTC(p[0], p[1], p[2]) >= cutoff).map(p => p[3])
  if (values.length < 2) return ''
  const min = Math.min(...values) - 15, max = Math.max(...values) + 15
  return values.map((value, index) => `${(index / (values.length - 1)) * 100},${100 - ((value - min) / (max - min)) * 100}`).join(' ')
})

function position(moves: string[], ply: number): Chess {
  const chess = new Chess()
  for (const move of moves.slice(0, ply)) { try { chess.move(move) } catch { break } }
  return chess
}
function gameResult(game: LichessGame): string { return game.winner ? game.winner === game.color ? 'win' : 'loss' : 'draw' }
function date(ms: number): string { return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' }) }
function info(text: string): void { message.value = text; error.value = '' }
function fail(cause: unknown): void { error.value = cause instanceof Error ? cause.message : String(cause); message.value = '' }
function playMoveSound(capture = false): void {
  if (!settings.value?.soundEnabled) return
  try {
    const context = new AudioContext()
    const oscillator = context.createOscillator(), gain = context.createGain()
    oscillator.type = 'sine'; oscillator.frequency.value = capture ? 390 : 520
    gain.gain.setValueAtTime(0.0001, context.currentTime)
    gain.gain.exponentialRampToValueAtTime(Math.max(0.001, settings.value.soundVolume * 0.07), context.currentTime + 0.005)
    gain.gain.exponentialRampToValueAtTime(0.0001, context.currentTime + 0.085)
    oscillator.connect(gain).connect(context.destination)
    oscillator.start(); oscillator.stop(context.currentTime + 0.09)
    oscillator.onended = () => { void context.close() }
  } catch { /* sound is optional */ }
}
async function run<T>(action: () => Promise<T>): Promise<T | undefined> {
  busy.value = true; error.value = ''
  try { return await action() } catch (cause) { fail(cause); return undefined }
  finally { busy.value = false }
}
function selectPage(next: Page): void { page.value = next; error.value = ''; message.value = '' }
function openSearch(): void { searchOpen.value = true; searchQuery.value = ''; searchIndex.value = 0; void nextTick(() => searchInput.value?.focus()) }
function chooseSearch(next: Page): void { selectPage(next); searchOpen.value = false }
async function save(): Promise<void> {
  if (!settings.value) return
  const saved = await run(() => window.kchess.saveSettings({ ...settings.value! }))
  if (saved) { settings.value = saved; info('Settings saved.'); void refreshEngine() }
}
async function refreshEngine(): Promise<void> {
  const result = await window.kchess.engineStatus(settings.value?.enginePath)
  engineReady.value = result.ready
}
async function chooseEngine(): Promise<void> {
  const path = await window.kchess.chooseEngine()
  if (path && settings.value) { settings.value.enginePath = path; await save() }
}
async function installEngine(): Promise<void> {
  const result = await run(() => window.kchess.installEngine())
  if (result && settings.value) { settings.value.enginePath = result.path; await save(); info(`Stockfish ${result.version} is ready.`) }
}
async function addAccount(): Promise<void> {
  const result = await run(() => window.kchess.addAccount(usernameInput.value))
  if (result) { data.value = result; usernameInput.value = ''; selectedAccount.value ||= result.accounts[0]?.username ?? ''; await sync(result.accounts.at(-1)?.username) }
}
async function removeAccount(username: string): Promise<void> {
  const result = await run(() => window.kchess.removeAccount(username))
  if (result) { data.value = result; selectedAccount.value = result.accounts[0]?.username ?? ''; info('Account removed.') }
}
async function connect(): Promise<void> {
  const result = await run(() => window.kchess.connectLichess())
  if (result) { data.value = result; selectedAccount.value = result.accounts.find(a => a.connected)?.username ?? ''; info('Lichess account connected.'); await sync(selectedAccount.value) }
}
async function sync(username?: string): Promise<void> {
  const result = await run(() => window.kchess.syncGames(username))
  if (result) { data.value = result; info('Lichess games synced.'); void loadProfile() }
}
async function loadProfile(): Promise<void> {
  if (!selectedAccount.value) { profile.value = null; ratingHistories.value = []; return }
  try {
    const [p, history] = await Promise.all([window.kchess.profile(selectedAccount.value), window.kchess.ratingHistory(selectedAccount.value)])
    profile.value = p
    ratingHistories.value = history
    if (!history.find(item => item.name === chartMode.value)?.points.length) chartMode.value = history.find(item => item.points.length)?.name ?? 'Blitz'
  } catch (cause) { fail(cause) }
}
watch(selectedAccount, () => { void loadProfile() })
watch([historyResult, historyRated, historyAccount, historyPageSize], () => { historyPage.value = 0 })
watch(reviewPlaying, playing => {
  if (playing) {
    const timer = setInterval(() => {
      if (!reviewPlaying.value || reviewPly.value >= reviewHistory.value.length) { clearInterval(timer); reviewPlaying.value = false }
      else reviewPly.value++
    }, 1200)
  }
})
watch(() => settings.value?.appearance, mode => {
  const dark = mode === 'dark' || (mode === 'system' && matchMedia('(prefers-color-scheme: dark)').matches)
  document.documentElement.classList.toggle('dark', dark)
}, { immediate: true })

function makeMove(uci: string): void {
  if (localPly.value !== localMoves.value.length || localGame.value.isGameOver() || thinking.value) return
  try {
    const game = position(localMoves.value, localMoves.value.length)
    const played = game.move({ from: uci.slice(0, 2) as Square, to: uci.slice(2, 4) as Square, promotion: uci[4] ?? 'q' })
    localMoves.value = [...localMoves.value, uci]
    localPly.value = localMoves.value.length
    playMoveSound(Boolean(played.captured))
    void computerTurn()
  } catch (cause) { fail(cause) }
}
async function computerTurn(): Promise<void> {
  const turn = localGame.value.turn() === 'w' ? 'white' : 'black'
  if (turn === userColor.value || localGame.value.isGameOver() || !engineReady.value) return
  const epoch = gameEpoch.value, moveCount = localMoves.value.length
  thinking.value = true
  try {
    const move = await window.kchess.bestMove([...localMoves.value], level.value, settings.value?.enginePath)
    if (epoch !== gameEpoch.value || moveCount !== localMoves.value.length) return
    const game = position(localMoves.value, localMoves.value.length)
    const played = game.move({ from: move.slice(0, 2) as Square, to: move.slice(2, 4) as Square, promotion: move[4] ?? 'q' })
    localMoves.value = [...localMoves.value, move]
    localPly.value = localMoves.value.length
    playMoveSound(Boolean(played.captured))
  } catch (cause) { fail(cause) }
  finally { if (epoch === gameEpoch.value) thinking.value = false }
}
function newGame(): void { gameEpoch.value++; thinking.value = false; localMoves.value = []; localPly.value = 0; flipped.value = userColor.value === 'black'; void computerTurn() }
function takeback(): void { if (localMoves.value.length) { gameEpoch.value++; thinking.value = false; localMoves.value = localMoves.value.slice(0, -2); localPly.value = localMoves.value.length } }
function localStatus(): string {
  if (localGame.value.isCheckmate()) return 'Checkmate'
  if (localGame.value.isDraw()) return 'Draw'
  if (localGame.value.isCheck()) return 'Check'
  return thinking.value ? 'Computer is thinking…' : `Your move · ${localGame.value.turn() === 'w' ? 'White' : 'Black'} to play`
}
function openReview(game: LichessGame): void { reviewPlaying.value = false; reviewGame.value = game; reviewPly.value = reviewMoves.value.length; page.value = 'history' }

async function startOnline(): Promise<void> {
  const result = await run(() => window.kchess.startOnline({ minutes: onlineMinutes.value, increment: onlineIncrement.value, color: onlineChoice.value, target: onlineTarget.value || undefined }))
  if (result) { onlinePhase.value = 'seeking'; onlineStatus.value = result.url ? 'Challenge sent. Waiting for acceptance…' : 'Looking for an opponent…' }
}
async function stopOnline(): Promise<void> { await window.kchess.cancelOnline(); onlinePhase.value = 'idle'; onlineStatus.value = '' }
function readOnlineEvent(event: OnlineEvent): void {
  if (event.type === 'gameStart' && event.game?.id) {
    onlineId.value = event.game.id; onlineColor.value = event.game.color ?? 'white'
    onlineOpponent.value = event.game.opponent?.username ?? 'Opponent'; onlinePhase.value = 'playing'; onlineStatus.value = 'Game in progress'
  }
  const state = event.type === 'gameFull' ? event.state : event
  if ((event.type === 'gameFull' || event.type === 'gameState') && state) {
    const previousCount = onlineMoves.value.length
    if (event.id) onlineId.value = event.id
    onlineMoves.value = String(state.moves ?? '').trim().split(/\s+/).filter(Boolean)
    if (previousCount && onlineMoves.value.length > previousCount) playMoveSound()
    onlineClock.value = { white: Number(state.wtime ?? 0), black: Number(state.btime ?? 0) }
    onlineStart.value = Date.now()
    if (event.type === 'gameFull') {
      const ours = data.value?.accounts.find(a => a.connected)?.username.toLowerCase()
      onlineColor.value = event.white?.id?.toLowerCase() === ours ? 'white' : 'black'
      onlineOpponent.value = onlineColor.value === 'white' ? event.black?.name ?? 'Opponent' : event.white?.name ?? 'Opponent'
    }
    if (state.status && state.status !== 'started') { onlinePhase.value = 'finished'; onlineStatus.value = `Game ended: ${state.status}` }
    else onlinePhase.value = 'playing'
  }
}
async function onlineMove(uci: string): Promise<void> {
  if (!onlineId.value || onlinePhase.value !== 'playing') return
  try { await window.kchess.playOnline(onlineId.value, uci) } catch (cause) { fail(cause) }
}
async function onlineAction(action: 'resign' | 'abort' | 'takeback' | 'declineTakeback'): Promise<void> {
  if (!onlineId.value) return
  await run(() => window.kchess.onlineAction(onlineId.value, action))
}
function clock(color: 'white' | 'black'): string {
  void now.value
  const base = onlineClock.value[color]
  const active = onlineGame.value.turn() === (color === 'white' ? 'w' : 'b') && onlinePhase.value === 'playing'
  const remaining = Math.max(0, base - (active ? Date.now() - onlineStart.value : 0))
  const seconds = Math.floor(remaining / 1000)
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}
const now = ref(Date.now())
function keydown(event: KeyboardEvent): void {
  if ((event.metaKey || event.ctrlKey) && event.key === ',') { event.preventDefault(); selectPage('settings') }
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'f') { event.preventDefault(); openSearch(); return }
  if (searchOpen.value) {
    if (event.key === 'Escape') { searchOpen.value = false; event.preventDefault() }
    if (event.key === 'ArrowDown') { searchIndex.value = (searchIndex.value + 1) % Math.max(1, searchResults.value.length); event.preventDefault() }
    if (event.key === 'ArrowUp') { searchIndex.value = (searchIndex.value - 1 + Math.max(1, searchResults.value.length)) % Math.max(1, searchResults.value.length); event.preventDefault() }
    if (event.key === 'Enter' && searchResults.value[searchIndex.value]) { chooseSearch(searchResults.value[searchIndex.value].id); event.preventDefault() }
    return
  }
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) return
  if (page.value === 'computer' && event.key === 'ArrowLeft') localPly.value = Math.max(0, localPly.value - 1)
  if (page.value === 'computer' && event.key === 'ArrowRight') localPly.value = Math.min(localMoves.value.length, localPly.value + 1)
}
onMounted(async () => {
  data.value = await window.kchess.loadData(); settings.value = { ...data.value.settings }
  selectedAccount.value = data.value.accounts[0]?.username ?? ''
  await refreshEngine()
  offOnline = window.kchess.onOnlineEvent(readOnlineEvent)
  offOnlineError = window.kchess.onOnlineError(fail)
  void window.kchess.resumeOnline().then(id => { if (id) { onlineId.value = id; onlinePhase.value = 'playing' } }).catch(() => undefined)
  ticker = setInterval(() => { now.value = Date.now() }, 250)
  window.addEventListener('keydown', keydown)
  void loadProfile()
})
onUnmounted(() => { reviewPlaying.value = false; offOnline?.(); offOnlineError?.(); if (ticker) clearInterval(ticker); window.removeEventListener('keydown', keydown) })
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="brand"><span class="brand-mark">♞</span><span class="brand-label">KChess</span></div>
      <div><div class="nav-caption">Workspace</div>
        <button v-for="item in nav.slice(0, 4)" :key="item.id" class="nav-button" :class="{ active: page === item.id }" :title="item.label" @click="selectPage(item.id)"><UIcon :name="item.icon" /><span>{{ item.label }}</span></button>
      </div>
      <div class="sidebar-bottom"><button class="nav-button" :class="{ active: page === 'settings' }" title="Settings" @click="selectPage('settings')"><UIcon name="i-lucide-settings-2" /><span>Settings</span></button></div>
    </aside>
    <main class="main-area">
      <header class="topbar"><div class="font-semibold text-sm">{{ nav.find(n => n.id === page)?.label }}</div><div class="flex items-center gap-3 text-xs muted"><UButton icon="i-lucide-search" variant="ghost" color="neutral" size="sm" @click="openSearch">Search <kbd>⌘ F</kbd></UButton><span v-if="selectedAccount">@{{ selectedAccount }}</span><UBadge v-if="busy" color="neutral" variant="soft">Working…</UBadge><span class="brand-mark" style="width:28px;height:28px;font-size:20px">♞</span></div></header>
      <div v-if="!data || !settings" class="empty">Loading KChess…</div>
      <div v-else class="page">
        <div v-if="error" class="notice error" role="alert">{{ error }}</div>
        <div v-else-if="message" class="notice" role="status">{{ message }}</div>

        <template v-if="page === 'dashboard'">
          <div class="flex justify-between items-start gap-3"><div><h1 class="page-title">Dashboard</h1><p class="page-subtitle">Your Lichess activity at a glance</p></div><UButton icon="i-lucide-refresh-cw" variant="outline" color="neutral" :loading="busy" @click="sync()">Sync games</UButton></div>
          <div v-if="!data.accounts.length" class="card empty" style="margin-top:24px"><div style="font-size:42px">♞</div><h2 class="section-title">Connect your chess world</h2><p>Add a Lichess username to see ratings, recent games, and your history.</p><UButton style="margin-top:14px" @click="selectPage('settings')">Open Settings</UButton></div>
          <template v-else>
            <div class="toolbar-row" style="margin-top:22px"><span class="muted text-xs">Account</span><select v-model="selectedAccount" class="rounded-md border border-default bg-default px-3 py-2 text-sm"><option v-for="account in data.accounts" :key="account.username">{{ account.username }}</option></select></div>
            <div class="stats-grid"><div class="card"><div class="stat-label">Games tracked</div><div class="stat-value">{{ profile?.count.all ?? games.filter(g => g.account === selectedAccount).length }}</div><div class="stat-label">Lichess lifetime</div></div><div class="card"><div class="stat-label">Blitz rating</div><div class="stat-value">{{ profile?.perfs?.blitz?.rating ?? '—' }}</div><div class="stat-label">{{ profile?.perfs?.blitz?.prog ? `${profile.perfs.blitz.prog > 0 ? '+' : ''}${profile.perfs.blitz.prog} recent` : 'Current rating' }}</div></div><div class="card"><div class="stat-label">Rapid rating</div><div class="stat-value">{{ profile?.perfs?.rapid?.rating ?? '—' }}</div><div class="stat-label">Current rating</div></div><div class="card"><div class="stat-label">Recent record</div><div class="stat-value" style="font-size:23px">{{ counts.win }} / {{ counts.loss }} / {{ counts.draw }}</div><div class="stat-label">Wins / losses / draws synced</div></div></div>
            <div class="two-columns"><div class="card"><div class="flex justify-between items-start gap-2"><h2 class="section-title">Rating history</h2><div class="toolbar-row"><select v-model="chartMode" class="rounded-md border border-default bg-default px-2 py-1 text-xs"><option v-for="mode in ['Bullet','Blitz','Rapid','Classical','Correspondence','Puzzle']" :key="mode">{{ mode }}</option></select><select v-model="chartRange" class="rounded-md border border-default bg-default px-2 py-1 text-xs"><option v-for="range in ['1M','3M','6M','YTD','1Y','All']" :key="range">{{ range }}</option></select></div></div><div v-if="chartPoints" style="height:240px;padding:16px 0"><svg viewBox="0 0 100 100" preserveAspectRatio="none" width="100%" height="100%" aria-label="Rating history chart"><line v-for="y in [0,25,50,75,100]" :key="y" x1="0" :y1="y" x2="100" :y2="y" stroke="currentColor" opacity=".1" stroke-width=".3"/><polyline :points="chartPoints" fill="none" stroke="#8baa69" stroke-width="1.5" vector-effect="non-scaling-stroke" /></svg></div><div v-else class="empty">No rating points for this mode and range.</div></div><div class="card"><h2 class="section-title">Recent games</h2><div v-if="!recentGames.length" class="empty">No synced games yet.</div><button v-for="game in recentGames" :key="game.id" class="game-row" @click="openReview(game)"><span :class="['result', gameResult(game)]">{{ gameResult(game).toUpperCase() }}</span><span>{{ game.opponent }}</span><span>{{ game.speed }}</span><span class="muted">{{ date(game.createdAt) }}</span></button></div></div>
          </template>
        </template>

        <template v-else-if="page === 'computer'">
          <div class="flex justify-between items-start gap-3"><div><h1 class="page-title">Play with Computer</h1><p class="page-subtitle">Challenge Stockfish at your level</p></div><div class="toolbar-row"><UButton variant="outline" color="neutral" icon="i-lucide-rotate-ccw" @click="newGame">New game</UButton><UButton variant="outline" color="neutral" icon="i-lucide-undo-2" :disabled="localMoves.length < 2" @click="takeback">Take back</UButton></div></div>
          <div class="toolbar-row" style="margin-top:20px"><span class="muted text-xs">Level</span><select v-model="level" class="rounded-md border border-default bg-default px-3 py-2 text-sm"><option value="low">Low · 1350</option><option value="medium">Medium · 1800</option><option value="high">High · Max</option></select><span class="muted text-xs">Play as</span><select v-model="userColor" class="rounded-md border border-default bg-default px-3 py-2 text-sm" @change="newGame"><option value="white">White</option><option value="black">Black</option></select><UBadge :color="engineReady ? 'success' : 'warning'" variant="soft">{{ engineReady ? 'Stockfish ready' : 'Set Stockfish in Settings' }}</UBadge></div>
          <div class="play-layout"><div class="board-stack"><div class="player-line"><span><span class="player-avatar">♞</span>Stockfish · {{ level }}</span><span class="muted">{{ thinking ? 'Thinking…' : '' }}</span></div><ChessBoard :fen="localDisplay.fen()" :orientation="flipped ? 'black' : 'white'" :light="settings.lightSquare" :dark="settings.darkSquare" :interactive="engineReady && localPly === localMoves.length && localGame.turn() === (userColor === 'white' ? 'w' : 'b')" :last-move="localLast ? `${localLast.from}${localLast.to}` : ''" @move="makeMove"/><div class="player-line"><span><span class="player-avatar">♙</span>You · {{ userColor }}</span><span class="muted">{{ localStatus() }}</span></div></div><div class="card"><h2 class="section-title">Moves</h2><div class="moves-list"><template v-for="index in Math.ceil(localHistory.length / 2)" :key="index"><div class="move-cell muted">{{ index }}.</div><button class="move-cell text-left" :class="{ active: localPly === index * 2 - 1 }" @click="localPly = index * 2 - 1">{{ localHistory[index * 2 - 2]?.san }}</button><button class="move-cell text-left" :class="{ active: localPly === index * 2 }" @click="localPly = index * 2">{{ localHistory[index * 2 - 1]?.san ?? '' }}</button></template><div v-if="!localHistory.length" class="empty" style="grid-column:span 3">Your moves will appear here.</div></div><div class="move-controls"><UButton size="sm" variant="ghost" color="neutral" icon="i-lucide-skip-back" :disabled="localPly === 0" @click="localPly = 0"/><UButton size="sm" variant="ghost" color="neutral" icon="i-lucide-chevron-left" :disabled="localPly === 0" @click="localPly--"/><UButton size="sm" variant="ghost" color="neutral" icon="i-lucide-chevron-right" :disabled="localPly === localMoves.length" @click="localPly++"/><UButton size="sm" variant="ghost" color="neutral" icon="i-lucide-skip-forward" :disabled="localPly === localMoves.length" @click="localPly = localMoves.length"/><UButton size="sm" variant="ghost" color="neutral" icon="i-lucide-refresh-cw" @click="flipped = !flipped"/></div></div></div>
        </template>

        <template v-else-if="page === 'online'">
          <h1 class="page-title">Play Online</h1><p class="page-subtitle">Play live games through your connected Lichess account</p>
          <div v-if="!data.accounts.some(a => a.connected)" class="card empty" style="margin-top:24px"><div style="font-size:42px">♟</div><h2 class="section-title">Connect Lichess to play</h2><p>Online play uses Lichess OAuth and the Board API.</p><UButton style="margin-top:14px" @click="selectPage('settings')">Open Settings</UButton></div>
          <div v-else-if="onlinePhase === 'idle'" class="card" style="margin-top:24px;max-width:600px"><h2 class="section-title">Find a game</h2><div class="toolbar-row"><div class="settings-field"><span>Time control</span><select v-model.number="onlineMinutes" class="rounded-md border border-default bg-default px-3 py-2 text-sm"><option :value="10">10 minutes</option><option :value="15">15 minutes</option><option :value="30">30 minutes</option><option :value="60">60 minutes</option></select></div><div class="settings-field"><span>Increment</span><select v-model.number="onlineIncrement" class="rounded-md border border-default bg-default px-3 py-2 text-sm"><option :value="0">0 seconds</option><option :value="5">5 seconds</option><option :value="10">10 seconds</option><option :value="20">20 seconds</option></select></div><div class="settings-field"><span>Color</span><select v-model="onlineChoice" class="rounded-md border border-default bg-default px-3 py-2 text-sm"><option value="random">Random</option><option value="white">White</option><option value="black">Black</option></select></div></div><div class="settings-field"><span>Challenge a user (optional)</span><UInput v-model="onlineTarget" placeholder="Lichess username" /></div><UButton icon="i-lucide-play" :loading="busy" @click="startOnline">{{ onlineTarget ? 'Send challenge' : 'Find opponent' }}</UButton></div>
          <div v-else-if="onlinePhase === 'seeking'" class="card empty" style="margin-top:24px"><UIcon name="i-lucide-loader-circle" class="text-3xl"/><p>{{ onlineStatus }}</p><UButton variant="outline" color="neutral" @click="stopOnline">Cancel</UButton></div>
          <div v-else class="play-layout"><div class="board-stack"><div class="player-line"><span><span class="player-avatar">♟</span>{{ onlineOpponent }}</span><span>{{ clock(onlineColor === 'white' ? 'black' : 'white') }}</span></div><ChessBoard :fen="onlineGame.fen()" :orientation="onlineColor" :light="settings.lightSquare" :dark="settings.darkSquare" :interactive="onlinePhase === 'playing' && onlineGame.turn() === (onlineColor === 'white' ? 'w' : 'b')" @move="onlineMove"/><div class="player-line"><span><span class="player-avatar">♙</span>You</span><span>{{ clock(onlineColor) }}</span></div></div><div class="card"><h2 class="section-title">Live game</h2><p class="muted text-sm">{{ onlineStatus }}</p><p class="muted text-xs">Game {{ onlineId }}</p><div class="moves-list" style="height:300px;grid-template-columns:36px 1fr 1fr"><template v-for="index in Math.ceil(onlineGame.history().length / 2)" :key="index"><span class="move-cell muted">{{ index }}.</span><span class="move-cell">{{ onlineGame.history()[index * 2 - 2] }}</span><span class="move-cell">{{ onlineGame.history()[index * 2 - 1] }}</span></template></div><div class="toolbar-row" style="margin-top:14px"><UButton size="sm" variant="outline" color="neutral" @click="onlineAction('takeback')">Takeback</UButton><UButton size="sm" variant="outline" color="error" @click="onlineAction('resign')">Resign</UButton><UButton size="sm" variant="outline" color="neutral" @click="stopOnline">Leave</UButton></div></div></div>
        </template>

        <template v-else-if="page === 'history'">
          <div class="flex justify-between items-start gap-3"><div><h1 class="page-title">History</h1><p class="page-subtitle">Review your synced Lichess games</p></div><UButton icon="i-lucide-refresh-cw" variant="outline" color="neutral" @click="sync()">Sync games</UButton></div>
          <template v-if="reviewGame"><div class="toolbar-row" style="margin-top:20px"><UButton icon="i-lucide-arrow-left" variant="ghost" color="neutral" @click="reviewPlaying = false; reviewGame = null">All games</UButton><span class="muted text-sm">{{ reviewGame.account }} vs {{ reviewGame.opponent }} · {{ date(reviewGame.createdAt) }}</span></div><div class="play-layout"><div class="board-stack"><ChessBoard :fen="reviewPosition.fen()" :orientation="reviewGame.color" :light="settings.lightSquare" :dark="settings.darkSquare" /></div><div class="card"><h2 class="section-title">{{ reviewGame.opening ?? 'Game review' }}</h2><div class="moves-list"><template v-for="index in Math.ceil(reviewHistory.length / 2)" :key="index"><span class="move-cell muted">{{ index }}.</span><button class="move-cell text-left" :class="{ active: reviewPly === index * 2 - 1 }" @click="reviewPly = index * 2 - 1">{{ reviewHistory[index * 2 - 2] }}</button><button class="move-cell text-left" :class="{ active: reviewPly === index * 2 }" @click="reviewPly = index * 2">{{ reviewHistory[index * 2 - 1] }}</button></template></div><div class="move-controls"><UButton size="sm" variant="ghost" color="neutral" icon="i-lucide-skip-back" @click="reviewPly = 0"/><UButton size="sm" variant="ghost" color="neutral" icon="i-lucide-chevron-left" :disabled="!reviewPly" @click="reviewPly--"/><UButton size="sm" variant="ghost" color="neutral" :icon="reviewPlaying ? 'i-lucide-pause' : 'i-lucide-play'" @click="reviewPlaying = !reviewPlaying"/><UButton size="sm" variant="ghost" color="neutral" icon="i-lucide-chevron-right" :disabled="reviewPly >= reviewHistory.length" @click="reviewPly++"/><UButton size="sm" variant="ghost" color="neutral" icon="i-lucide-skip-forward" @click="reviewPly = reviewHistory.length"/></div></div></div></template>
          <template v-else><div class="toolbar-row" style="margin-top:22px"><select v-model="historyAccount" class="rounded-md border border-default bg-default px-3 py-2 text-sm"><option value="all">All accounts</option><option v-for="a in data.accounts" :key="a.username">{{ a.username }}</option></select><select v-model="historyResult" class="rounded-md border border-default bg-default px-3 py-2 text-sm"><option value="all">All results</option><option value="win">Wins</option><option value="loss">Losses</option><option value="draw">Draws</option></select><select v-model="historyRated" class="rounded-md border border-default bg-default px-3 py-2 text-sm"><option value="all">Rated & casual</option><option value="rated">Rated</option><option value="casual">Casual</option></select><select v-model.number="historyPageSize" class="rounded-md border border-default bg-default px-3 py-2 text-sm"><option v-for="size in [10,20,30,40,50]" :key="size" :value="size">{{ size }} per page</option></select><span class="muted text-xs">{{ filteredGames.length }} games</span></div><div class="card" style="margin-top:16px"><div v-if="!visibleGames.length" class="empty">No games match these filters. Add an account or sync your history.</div><button v-for="game in visibleGames" :key="`${game.account}-${game.id}`" class="game-row" @click="openReview(game)"><span :class="['result', gameResult(game)]">{{ gameResult(game).toUpperCase() }}</span><span><strong>{{ game.opponent }}</strong><br><span class="muted">{{ game.opening ?? game.account }}</span></span><span>{{ game.speed }}<br><span class="muted">{{ game.rated ? 'Rated' : 'Casual' }}</span></span><span class="muted">{{ date(game.createdAt) }}</span></button><div class="move-controls"><UButton size="sm" variant="ghost" color="neutral" :disabled="historyPage === 0" @click="historyPage--">Previous</UButton><span class="muted text-xs" style="align-self:center">Page {{ historyPage + 1 }}</span><UButton size="sm" variant="ghost" color="neutral" :disabled="(historyPage + 1) * historyPageSize >= filteredGames.length" @click="historyPage++">Next</UButton></div></div></template>
        </template>

        <template v-else-if="page === 'settings'">
          <div class="flex justify-between items-start gap-3"><div><h1 class="page-title">Settings</h1><p class="page-subtitle">Make KChess yours</p></div><UButton icon="i-lucide-save" :loading="busy" @click="save">Save changes</UButton></div>
          <div class="settings-grid"><div class="card"><h2 class="section-title">Appearance</h2><div class="settings-field"><span>Theme</span><select v-model="settings.appearance" class="rounded-md border border-default bg-default px-3 py-2 text-sm"><option value="system">System</option><option value="light">Light</option><option value="dark">Dark</option></select></div><div class="settings-field"><span>Board colors</span><select v-model="settings.boardPreset" class="rounded-md border border-default bg-default px-3 py-2 text-sm" @change="settings.boardPreset === 'lichess' ? (settings.lightSquare = '#f0d9b5', settings.darkSquare = '#b58863') : settings.boardPreset === 'chess.com' ? (settings.lightSquare = '#eeeed2', settings.darkSquare = '#769656') : null"><option value="lichess">Lichess</option><option value="chess.com">Chess.com</option><option value="custom">Custom</option></select></div><div class="toolbar-row"><label class="settings-field">Light square<input v-model="settings.lightSquare" type="color" @input="settings.boardPreset = 'custom'"></label><label class="settings-field">Dark square<input v-model="settings.darkSquare" type="color" @input="settings.boardPreset = 'custom'"></label></div><div class="settings-field"><span>Sound</span><USwitch v-model="settings.soundEnabled" /></div><div class="settings-field"><span>Volume · {{ Math.round(settings.soundVolume * 100) }}%</span><input v-model.number="settings.soundVolume" type="range" min="0" max="1" step="0.05"></div></div>
            <div class="card"><h2 class="section-title">Chess Engine</h2><p class="muted text-xs">Use a local Stockfish executable to play against the computer.</p><div class="settings-field"><span>Stockfish path</span><UInput v-model="settings.enginePath" placeholder="Select a Stockfish executable" /></div><div class="toolbar-row"><UButton variant="outline" color="neutral" icon="i-lucide-download" :loading="busy" @click="installEngine">Install / update</UButton><UButton variant="outline" color="neutral" icon="i-lucide-folder-open" @click="chooseEngine">Choose file</UButton><UBadge :color="engineReady ? 'success' : 'warning'" variant="soft">{{ engineReady ? 'Ready' : 'Not found' }}</UBadge></div></div>
            <div class="card" style="grid-column:1/-1"><div class="flex justify-between items-start gap-3"><div><h2 class="section-title">Lichess Accounts</h2><p class="muted text-xs">Track public accounts for dashboard and history. Connect your own account for online play.</p></div><UButton icon="i-lucide-link" :loading="busy" @click="connect">Connect my account</UButton></div><div class="toolbar-row" style="margin:20px 0"><UInput v-model="usernameInput" placeholder="Lichess username" @keyup.enter="addAccount"/><UButton variant="outline" color="neutral" @click="addAccount">Add public account</UButton><UButton variant="ghost" color="neutral" :loading="busy" @click="sync()">Sync all</UButton></div><div v-if="!data.accounts.length" class="empty">No accounts added yet.</div><div v-for="account in data.accounts" :key="account.username" class="flex justify-between items-center border-t border-default py-3 text-sm"><div><strong>@{{ account.username }}</strong><UBadge class="ml-2" :color="account.connected ? 'success' : 'neutral'" variant="soft">{{ account.connected ? 'Connected' : 'Public' }}</UBadge><div class="muted text-xs">{{ account.lastSyncedAt ? `Last synced ${date(account.lastSyncedAt)}` : 'Not synced yet' }}</div></div><div class="toolbar-row"><UButton size="sm" variant="ghost" color="neutral" @click="sync(account.username)">Sync</UButton><UButton size="sm" variant="ghost" color="error" @click="removeAccount(account.username)">Remove</UButton></div></div></div></div>
        </template>
      </div>
    </main>
    <div v-if="searchOpen" class="search-backdrop" @click.self="searchOpen = false"><div class="search-dialog" role="dialog" aria-label="Navigate KChess"><input ref="searchInput" v-model="searchQuery" class="search-input" placeholder="Search pages…" aria-label="Search pages"/><button v-for="(item, index) in searchResults" :key="item.id" class="search-result" :class="{ active: index === searchIndex }" @click="chooseSearch(item.id)"><UIcon :name="item.icon"/><span>{{ item.label }}</span></button><div v-if="!searchResults.length" class="empty">No matching pages</div></div></div>
  </div>
</template>
