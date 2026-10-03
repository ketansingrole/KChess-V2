import { createInterface } from 'node:readline'
import { assertAnalysisRequest } from '../shared/validate'
import type { AnalysisRequest, AnalysisUpdate, EngineLine } from '../shared/types'
import { parseInfo } from '../shared/uciInfo'
import { engineStatus, engineThreads, spawnEngine } from './engine'

/**
 * Analysis keeps one Stockfish running between positions (unlike `bestMove`, which starts a fresh
 * process per move), so stepping through a game re-uses the hash and answers at once. A new
 * position never interrupts mid-output: it waits for the old search's `bestmove`, so every `info`
 * line is credited to the position it belongs to.
 */

/** Without “infinite”, stop at a depth that is strong and still quick with the bundled WASM build. */
const DEPTH_LIMIT = 26
const TIME_LIMIT_MS = 60_000
/** Emit at most this often; the board's arrows and lines do not need every info line. */
const EMIT_MS = 120
/** Free the engine's memory after a while without analysis. */
const IDLE_MS = 120_000

type Emit = (update: AnalysisUpdate) => void

interface Search {
  id: number
  request: AnalysisRequest
  whiteToMove: boolean
  lines: Map<number, EngineLine>
  nps?: number
}

interface Engine {
  key: string
  write: (command: string) => void
  kill: () => void
  ready: boolean
  searching: boolean
}

let engine: Engine | null = null
let current: Search | null = null
let pending: Search | null = null
let nextId = 0
let emitTimer: ReturnType<typeof setTimeout> | undefined
let idleTimer: ReturnType<typeof setTimeout> | undefined
let emit: Emit = () => undefined

function snapshot(search: Search, done: boolean, error?: string): AnalysisUpdate {
  const lines = [...search.lines.values()]
    .filter((line) => line.rank <= search.request.lines)
    .sort((a, b) => a.rank - b.rank)
  return {
    id: search.id,
    fen: search.request.fen,
    depth: lines[0]?.depth ?? 0,
    nps: search.nps,
    lines,
    done,
    ...(error ? { error } : {}),
  }
}

function flush(done = false): void {
  clearTimeout(emitTimer)
  emitTimer = undefined
  if (current) emit(snapshot(current, done))
}

function begin(): void {
  if (!engine?.ready || engine.searching || !pending) return
  clearTimeout(idleTimer)
  current = pending
  pending = null
  const { fen, lines, infinite } = current.request
  engine.write(`setoption name MultiPV value ${lines}`)
  engine.write(`position fen ${fen}`)
  engine.write(infinite ? 'go infinite' : `go depth ${DEPTH_LIMIT} movetime ${TIME_LIMIT_MS}`)
  engine.searching = true
}

function launch(key: string, status: Awaited<ReturnType<typeof engineStatus>>): Engine {
  const child = spawnEngine(status)
  const self: Engine = {
    key,
    ready: false,
    searching: false,
    write: (command) => {
      if (!child.stdin.destroyed) child.stdin.write(`${command}\n`)
    },
    kill: () => child.kill(),
  }
  const fail = (message: string): void => {
    if (engine !== self) return
    engine = null
    const last = pending ?? current
    current = pending = null
    clearTimeout(emitTimer)
    if (last) emit(snapshot(last, true, message))
  }
  child.on('error', (error) => fail(error.message))
  child.on('exit', () => fail('Stockfish stopped unexpectedly.'))
  child.stderr.resume()
  child.stdin.on('error', () => undefined)
  createInterface({ input: child.stdout }).on('line', (raw) => {
    if (engine !== self) return
    const line = raw.trim()
    if (line === 'uciok') {
      self.write(`setoption name Threads value ${engineThreads()}`)
      self.write('setoption name Hash value 128')
      self.write('isready')
    } else if (line === 'readyok' && !self.ready) {
      self.ready = true
      begin()
    } else if (line.startsWith('info ') && current && !pending) {
      const parsed = parseInfo(line, current.whiteToMove)
      if (!parsed || parsed.line.rank > current.request.lines) return
      current.lines.set(parsed.line.rank, parsed.line)
      if (parsed.nps !== undefined) current.nps = parsed.nps
      emitTimer ??= setTimeout(() => flush(), EMIT_MS)
    } else if (line.startsWith('bestmove')) {
      self.searching = false
      if (pending) {
        clearTimeout(emitTimer)
        emitTimer = undefined
      } else flush(true)
      current = null
      // A superseded search ends silently: the renderer has already moved on.
      if (pending) begin()
      else idleTimer = setTimeout(() => stopAnalysis(true), IDLE_MS)
    }
  })
  self.write('uci')
  return self
}

/** Analyse `raw` (validated here), replacing whatever was being analysed. Returns the update id. */
export async function startAnalysis(raw: unknown, configured: string, send: Emit): Promise<number> {
  const request = assertAnalysisRequest(raw)
  const status = await engineStatus(configured)
  if (!status.ready)
    throw new Error(
      'Stockfish could not be found. Reinstall KChess or choose an engine in Settings.',
    )
  emit = send
  const key = status.bundled ? 'bundled' : status.path
  // The engine was changed in Settings: start the new one.
  if (engine && engine.key !== key) stopAnalysis(true)
  engine ??= launch(key, status)
  const search: Search = {
    id: ++nextId,
    request,
    whiteToMove: request.fen.split(' ')[1] === 'w',
    lines: new Map(),
  }
  // Only one stop per running search: a request queued behind it simply replaces this one.
  if (engine.searching && !pending) engine.write('stop')
  pending = search
  begin()
  return search.id
}

/** The analysis board's engine is thinking. */
export const analysisRunning = (): boolean => Boolean(engine?.searching || pending)

/** Stop the search (its last update arrives with `done`); `kill` also ends the engine process. */
export function stopAnalysis(kill = false): void {
  pending = null
  clearTimeout(idleTimer)
  if (!engine) return
  if (kill) {
    const old = engine
    engine = null
    current = null
    clearTimeout(emitTimer)
    emitTimer = undefined
    old.kill()
  } else if (engine.searching) engine.write('stop')
}
