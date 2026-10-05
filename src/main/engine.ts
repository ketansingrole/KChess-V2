import { spawn, type ChildProcessByStdio } from 'node:child_process'
import { constants } from 'node:fs'
import { access, stat } from 'node:fs/promises'
import { join } from 'node:path'
import type { Readable, Writable } from 'node:stream'
import { app } from 'electron'
import { assertBestMoveOptions, assertMoves, type EngineLevel } from '../shared/validate'
import type { BestMoveOptions, EngineStatus } from '../shared/types'
import { MANAGED_PATH, managedEngine } from './managedEngine'
import { UciController, SearchCancelled, assertEngineAvailable } from './uci'
import { withEngineLease, searchThreads } from './engineScheduler'
import { replay } from '../shared/review'
import { INITIAL_FEN } from 'chessops/fen'
import { engineLevelInfo } from '../shared/engineLevels'

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
  const canDownload =
    ['darwin', 'win32', 'linux'].includes(process.platform) &&
    ['arm64', 'x64'].includes(process.arch)
  if (configured && (await isExecutableFile(configured)))
    return { ready: true, path: configured, bundled: false, managed, canDownload }
  try {
    await access(bundledEnginePath(), constants.R_OK)
    return { ready: true, path: '', bundled: true, managed, canDownload }
  } catch {
    return { ready: false, path: configured, bundled: false, managed, canDownload }
  }
}

/** Start the engine `engineStatus` found: the bundled WASM build under Electron's Node, or a native one. */
export function spawnEngine(
  status: EngineStatus,
): ChildProcessByStdio<Writable, Readable, Readable> {
  assertEngineAvailable()
  return status.bundled
    ? spawn(process.execPath, [bundledEnginePath()], {
        stdio: ['pipe', 'pipe', 'pipe'],
        env: { ...process.env, ELECTRON_RUN_AS_NODE: '1' },
      })
    : spawn(status.path, [], { stdio: ['pipe', 'pipe', 'pipe'] })
}

export const engineThreads = searchThreads
export const SUPERSEDED = 'Engine search superseded.'
let active: AbortController | undefined
let generation = 0
let lastMoveAt = 0
let warm: { key: string; uci: UciController } | undefined
let serial = Promise.resolve()
let idle: ReturnType<typeof setTimeout> | undefined
export const computerPlaying = (withinMs: number): boolean =>
  Boolean(active) || Date.now() - lastMoveAt < withinMs
export async function engineIdentity(status: EngineStatus): Promise<string> {
  if (status.bundled) return 'stockfish-19-lite'
  const file = await stat(status.path)
  return `${status.path}:${file.size}:${file.mtimeMs}`
}
export function stopEngine(close = false): void {
  generation++
  active?.abort()
  if (close) {
    warm?.uci.close()
    warm = undefined
    clearTimeout(idle)
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
  if (replay(fen ?? INITIAL_FEN, moves).length !== moves.length + 1)
    throw new Error('Invalid computer position or move history.')
  stopEngine()
  const epoch = generation
  const controller = new AbortController()
  active = controller
  clearTimeout(idle)
  lastMoveAt = Date.now()
  let resolveResult!: (move: string) => void
  let rejectResult!: (error: Error) => void
  const result = new Promise<string>((resolve, reject) => {
    resolveResult = resolve
    rejectResult = reject
  })
  serial = serial
    .catch(() => {})
    .then(async () => {
      try {
        controller.signal.throwIfAborted()
        const status = await engineStatus(configured)
        if (!status.ready)
          throw new Error('Stockfish could not be found. Choose an engine in Settings.')
        const key = await engineIdentity(status)
        if (epoch !== generation) throw new SearchCancelled()
        if (warm?.key !== key || warm.uci.failed) {
          warm?.uci.close()
          warm = { key, uci: new UciController(spawnEngine(status)) }
        }
        const target = warm.uci
        await withEngineLease(
          3,
          () => controller.abort(),
          controller.signal,
          async () => {
            await target.ready
            controller.signal.throwIfAborted()
            const profile = engineLevelInfo(level)
            target.write('ucinewgame')
            target.write(`setoption name Threads value ${searchThreads()}`)
            target.write('setoption name Hash value 64')
            // Set every time: the warm engine may have played a Chess960 game before.
            target.write(`setoption name UCI_Chess960 value ${Boolean(chess960)}`)
            target.write(`setoption name UCI_LimitStrength value ${Boolean(profile.uciElo)}`)
            if (profile.uciElo) target.write(`setoption name UCI_Elo value ${profile.uciElo}`)
            else target.write(`setoption name Skill Level value ${profile.skill ?? 20}`)
            await target.sync()
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
        rejectResult(
          controller.signal.aborted || cause instanceof SearchCancelled
            ? new Error(SUPERSEDED)
            : cause instanceof Error
              ? cause
              : new Error(String(cause)),
        )
      } finally {
        if (active === controller) {
          active = undefined
          idle = setTimeout(() => {
            warm?.uci.close()
            warm = undefined
          }, 120_000)
          idle.unref()
        }
      }
    })
  return result
}
