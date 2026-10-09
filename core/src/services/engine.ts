import { scopedState, platform } from './platform'
import { spawn, type ChildProcessByStdio } from 'node:child_process'
import { constants } from 'node:fs'
import { access, stat } from 'node:fs/promises'
import type { Readable, Writable } from 'node:stream'
import { assertBestMoveOptions, assertMoves, type EngineLevel } from '../domain/validate'
import type { BestMoveOptions, EngineStatus } from '../contracts/types'
import { managedEngine, managedPath } from './managedEngine'
import { UciController, SearchCancelled, assertEngineAvailable, ensureEngineOptions } from './uci'
import { withEngineLease, searchThreads } from './engineScheduler'
import { INITIAL_FEN } from 'chessops/fen'
import { engineLevelInfo } from '../domain/engineLevels'
import { errorSummary, logDebug, logWarn, truncateForLog } from './logger'
import { replayPositions } from './rules'

/**
 * Stockfish ships with the app as the `stockfish` npm package's lite
 * multi-threaded WASM build, run as a UCI process by the host's own Node
 * runtime. A native executable picked in Settings takes precedence.
 */
export const BUNDLED_ENGINE_SCRIPT = 'node_modules/stockfish/bin/stockfish-19-lite.js'

const bundledEnginePath = (): string => platform().bundledEnginePath

/**
 * Engine paths the renderer may persist in settings: the downloaded engine
 * KChess manages, or one the user picked in the native file dialog this
 * session (or that is already saved).
 */

export const trustEnginePath = (path: string): void => void serviceState.trusted.add(path)
export const isTrustedEnginePath = (path: string): boolean =>
  path === managedPath() || serviceState.trusted.has(path)

async function isExecutableFile(path: string): Promise<boolean> {
  try {
    await access(path, constants.X_OK)
    return (await stat(path)).isFile()
  } catch (cause) {
    logDebug('engine', 'Engine executable check failed:', path, cause)
    return false
  }
}

export async function engineStatus(configured = ''): Promise<EngineStatus> {
  const managed = await managedEngine()
  const canDownload =
    ['darwin', 'win32', 'linux'].includes(process.platform) &&
    ['arm64', 'x64'].includes(process.arch)
  if (configured && (await isExecutableFile(configured)))
    return { ready: true, path: configured, bundled: false, managed, canDownload }
  try {
    await access(bundledEnginePath(), constants.R_OK)
    return { ready: true, path: '', bundled: true, managed, canDownload }
  } catch (cause) {
    logDebug('engine', 'Bundled engine check failed:', cause)
    return { ready: false, path: configured, bundled: false, managed, canDownload }
  }
}

/** Start the engine `engineStatus` found: the bundled WASM build under the host's Node, or a native one. */
export function spawnEngine(
  status: EngineStatus,
): ChildProcessByStdio<Writable, Readable, Readable> {
  assertEngineAvailable()
  platform()
  return status.bundled
    ? spawn(process.execPath, [bundledEnginePath()], {
        stdio: ['pipe', 'pipe', 'pipe'],
        env: { ...process.env, ...platform().nodeEnv },
      })
    : spawn(status.path, [], { stdio: ['pipe', 'pipe', 'pipe'] })
}

export const engineThreads = searchThreads
export const SUPERSEDED = 'Engine search superseded.'

export const computerPlaying = (withinMs: number): boolean =>
  Boolean(serviceState.active) || Date.now() - serviceState.lastMoveAt < withinMs
export async function engineIdentity(status: EngineStatus): Promise<string> {
  if (status.bundled) return 'stockfish-19-lite'
  const file = await stat(status.path)
  return `${status.path}:${file.size}:${file.mtimeMs}`
}
export function stopEngine(close = false): void {
  serviceState.generation++
  serviceState.active?.abort()
  if (close) {
    serviceState.warm?.uci.close()
    serviceState.warm = undefined
    clearTimeout(serviceState.idle)
  }
}
export async function bestMove(
  moveList: unknown,
  level: EngineLevel,
  configured = '',
  options: BestMoveOptions = {},
): Promise<string> {
  const moves = assertMoves(moveList)
  const { fen, movetime, chess960 } = assertBestMoveOptions(options)
  if (replayPositions(fen ?? INITIAL_FEN, moves).length !== moves.length + 1)
    throw new Error('Invalid computer position or move history.')
  stopEngine()
  const epoch = serviceState.generation
  const controller = new AbortController()
  serviceState.active = controller
  clearTimeout(serviceState.idle)
  serviceState.lastMoveAt = Date.now()
  let resolveResult!: (move: string) => void
  let rejectResult!: (error: Error) => void
  const result = new Promise<string>((resolve, reject) => {
    resolveResult = resolve
    rejectResult = reject
  })
  serviceState.serial = serviceState.serial
    .catch((error: unknown) => {
      logDebug('engine', 'Previous engine search failed:', error)
    })
    .then(async () => {
      let key = ''
      try {
        controller.signal.throwIfAborted()
        const status = await engineStatus(configured)
        if (!status.ready)
          throw new Error('Stockfish could not be found. Choose an engine in Settings.')
        key = await engineIdentity(status)
        if (epoch !== serviceState.generation) throw new SearchCancelled()
        if (serviceState.warm?.key !== key || serviceState.warm.uci.failed) {
          serviceState.warm?.uci.close()
          serviceState.warm = { key, uci: new UciController(spawnEngine(status)) }
        }
        const target = serviceState.warm.uci
        await withEngineLease(
          3,
          () => controller.abort(),
          controller.signal,
          async () => {
            await target.ready
            controller.signal.throwIfAborted()
            const profile = engineLevelInfo(level)
            target.write('ucinewgame')
            // The warm process reuses the previous game's options; only changed values
            // are sent, so consecutive moves at the same level skip the round-trip
            // (and a redundant Hash clear). Chess960 is set every time in the cache:
            // the warm engine may have played a Chess960 game before.
            await ensureEngineOptions(target, {
              Threads: String(searchThreads()),
              Hash: '64',
              UCI_Chess960: String(Boolean(chess960)),
              UCI_LimitStrength: String(Boolean(profile.uciElo)),
              ...(profile.uciElo
                ? { UCI_Elo: String(profile.uciElo) }
                : { 'Skill Level': String(profile.skill ?? 20) }),
            })
            controller.signal.throwIfAborted()
            target.write(
              `position ${fen ? `fen ${fen}` : 'startpos'}${moves.length ? ` moves ${moves.join(' ')}` : ''}`,
            )
            const random = Math.random() < (profile.randomMove ?? 0)
            const legal: string[] = []
            const response = await target.search(
              random ? 'go perft 1' : `go movetime ${movetime ?? profile.time}`,
              (line) => {
                const found = /^([a-h][1-8][a-h][1-8][nbrq]?): \d+$/.exec(line)
                if (found) legal.push(found[1]!)
              },
              Math.max(15_000, (movetime ?? profile.time) + 10_000),
              controller.signal,
            )
            const move = random
              ? legal[Math.floor(Math.random() * legal.length)]
              : response.split(/\s+/)[1]
            if (!move || move === '(none)' || move === '0000')
              throw new Error('Stockfish found no legal move.')
            resolveResult(move)
          },
        )
      } catch (cause) {
        const context = [
          `level=${level}`,
          `moves=${moves.length}`,
          `engine=${truncateForLog(key || 'unstarted', 80)}`,
          fen ? `fen=${truncateForLog(fen, 60)}` : 'startpos',
        ].join(' ')
        if (controller.signal.aborted || cause instanceof SearchCancelled)
          logDebug('engine', 'Engine search superseded:', context, errorSummary(cause))
        else logWarn('engine', 'Engine search failed:', context, errorSummary(cause))
        rejectResult(
          controller.signal.aborted || cause instanceof SearchCancelled
            ? new Error(SUPERSEDED)
            : cause instanceof Error
              ? cause
              : new Error(String(cause)),
        )
      } finally {
        if (serviceState.active === controller) {
          serviceState.active = undefined
          serviceState.idle = setTimeout(() => {
            serviceState.warm?.uci.close()
            serviceState.warm = undefined
          }, 120_000)
          serviceState.idle.unref()
        }
      }
    })
  return result
}

export function resetEngine(): void {
  stopEngine(true)
  serviceState.trusted.clear()
  serviceState.lastMoveAt = 0
}

const serviceState = scopedState(() => ({
  trusted: new Set<string>(),
  active: undefined as AbortController | undefined,
  generation: 0,
  lastMoveAt: 0,
  warm: undefined as { key: string; uci: UciController } | undefined,
  serial: Promise.resolve(),
  idle: undefined as ReturnType<typeof setTimeout> | undefined,
}))
