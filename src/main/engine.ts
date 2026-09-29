import { spawn, type ChildProcess } from 'node:child_process'
import { constants } from 'node:fs'
import { access, stat } from 'node:fs/promises'
import { cpus } from 'node:os'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { app } from 'electron'
import { assertMoves, type EngineLevel } from '../shared/validate'
import type { EngineStatus } from '../shared/types'
import { MANAGED_PATH, managedEngine } from './managedEngine'

/**
 * Stockfish ships with the app as the `stockfish` npm package's lite
 * multi-threaded WASM build, run as a UCI process by Electron's own Node
 * runtime. A native executable picked in Settings takes precedence.
 */
const BUNDLED_SCRIPT = 'node_modules/stockfish/bin/stockfish-19-lite.js'

/** Where the bundled engine lives; packaged apps keep it outside the asar so a child process can load its .wasm. */
function bundledEnginePath(): string {
  return join(app.getAppPath(), BUNDLED_SCRIPT).replace(/app\.asar([\\/])/, 'app.asar.unpacked$1')
}

/**
 * Engine paths the renderer may persist in settings: the downloaded engine
 * KChess manages, or one the user picked in the native file dialog this
 * session (or that is already saved).
 */
const trusted = new Set<string>()
export const trustEnginePath = (path: string): void => void trusted.add(path)
export const isTrustedEnginePath = (path: string): boolean =>
  path === MANAGED_PATH || trusted.has(path)

async function isExecutableFile(path: string): Promise<boolean> {
  try {
    await access(path, constants.X_OK)
    return (await stat(path)).isFile()
  } catch {
    return false
  }
}

export async function engineStatus(configured = ''): Promise<EngineStatus> {
  const managed = await managedEngine()
  const canDownload = process.platform === 'darwin'
  if (configured && (await isExecutableFile(configured)))
    return { ready: true, path: configured, bundled: false, managed, canDownload }
  try {
    await access(bundledEnginePath(), constants.R_OK)
    return { ready: true, path: '', bundled: true, managed, canDownload }
  } catch {
    return { ready: false, path: configured, bundled: false, managed, canDownload }
  }
}

const PROFILES: Record<EngineLevel, { elo: number; time: number }> = {
  low: { elo: 1350, time: 700 },
  medium: { elo: 1800, time: 1400 },
  high: { elo: 0, time: 2500 },
}

export const SUPERSEDED = 'Engine search superseded.'

let active: { child: ChildProcess; cancel: (reason: Error) => void } | null = null

/** Kill any running search (new game, takeback, app quit). */
export function stopEngine(): void {
  active?.cancel(new Error(SUPERSEDED))
}

export async function bestMove(
  moveList: unknown,
  level: EngineLevel,
  configured = '',
): Promise<string> {
  const moves = assertMoves(moveList)
  const status = await engineStatus(configured)
  if (!status.ready)
    throw new Error(
      'Stockfish could not be found. Reinstall KChess or choose an engine in Settings.',
    )
  const profile = PROFILES[level]
  // A newer request makes any in-flight search stale; the renderer drops stale results.
  stopEngine()
  return new Promise((resolve, reject) => {
    const child = status.bundled
      ? spawn(process.execPath, [bundledEnginePath()], {
          stdio: ['pipe', 'pipe', 'pipe'],
          env: { ...process.env, ELECTRON_RUN_AS_NODE: '1' },
        })
      : spawn(status.path, [], { stdio: ['pipe', 'pipe', 'pipe'] })
    let phase: 'uci' | 'ready' | 'search' = 'uci'
    let settled = false
    const finish = (error?: Error, move?: string): void => {
      if (settled) return
      settled = true
      clearTimeout(timeout)
      if (active?.child === child) active = null
      child.kill()
      if (error) reject(error)
      else resolve(move ?? '')
    }
    const timeout = setTimeout(() => finish(new Error('Stockfish timed out.')), 15_000)
    active = { child, cancel: (reason) => finish(reason) }
    child.on('error', (error) => finish(error))
    child.on('exit', (code) => {
      if (!settled) finish(new Error(`Stockfish exited (${code}).`))
    })
    // Drain stderr so a chatty engine can never block on a full pipe.
    child.stderr.resume()
    child.stdin.on('error', () => undefined)
    createInterface({ input: child.stdout }).on('line', (raw) => {
      const line = raw.trim()
      if (phase === 'uci' && line === 'uciok') {
        child.stdin.write(
          `setoption name Threads value ${Math.max(1, Math.min(4, cpus().length - 1))}\n`,
        )
        child.stdin.write('setoption name Hash value 64\n')
        if (profile.elo) {
          child.stdin.write('setoption name UCI_LimitStrength value true\n')
          child.stdin.write(`setoption name UCI_Elo value ${profile.elo}\n`)
        } else child.stdin.write('setoption name UCI_LimitStrength value false\n')
        child.stdin.write('isready\n')
        phase = 'ready'
      } else if (phase === 'ready' && line === 'readyok') {
        child.stdin.write(`position startpos${moves.length ? ` moves ${moves.join(' ')}` : ''}\n`)
        child.stdin.write(`go movetime ${profile.time}\n`)
        phase = 'search'
      } else if (phase === 'search' && line.startsWith('bestmove ')) {
        const move = line.split(/\s+/)[1]
        if (!move || move === '(none)') finish(new Error('Stockfish found no legal move.'))
        else finish(undefined, move)
      }
    })
    child.stdin.write('uci\n')
  })
}
