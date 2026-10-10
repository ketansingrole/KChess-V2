import { nativeCall, nativeCallSync } from './nativeCore.ts'
import type { BestMoveOptions, EngineStatus } from '../contracts/types'
import type { EngineLevel } from '../domain/validate'

/**
 * The computer's engine, run by the Rust core (`crates/kchess-core/src/engine`). These wrappers
 * keep the TypeScript API the frontends and the session services use; the search, its warm
 * process, the status and the trusted paths live in Rust.
 */

/**
 * Stockfish ships with the app as the `stockfish` npm package's lite
 * multi-threaded WASM build, run as a UCI process by the host's own Node
 * runtime. A native executable picked in Settings takes precedence.
 */
export const BUNDLED_ENGINE_SCRIPT = 'node_modules/stockfish/bin/stockfish-19-lite.js'

export const SUPERSEDED = 'Engine search superseded.'

/** A search ended on purpose (`SearchCancelled`); `isExpectedCancellation` recognises it. */
export class SearchCancelled extends Error {
  constructor() {
    super('Engine search cancelled.')
    this.name = 'SearchCancelled'
  }
}

/** Engine paths the renderer may persist in settings (`engine.trust`). */
export const trustEnginePath = (path: string): void => {
  nativeCallSync('engine.trust', path)
}
export const isTrustedEnginePath = (path: string): boolean =>
  nativeCallSync<boolean>('engine.isTrusted', path)

export const engineStatus = (configured = ''): Promise<EngineStatus> =>
  nativeCall<EngineStatus>('engine.status', configured)

/** The engine budget a search may use (`engine.threads`). */
export const engineThreads = (): number => nativeCallSync<number>('engine.threads')

export const computerPlaying = (withinMs: number): boolean =>
  nativeCallSync<boolean>('engine.computerPlaying', withinMs)

export const engineIdentity = (status: EngineStatus): Promise<string> =>
  nativeCall<string>('engine.identity', status)

/** Supersedes the search running; with `close`, also ends the warm engine. */
export function stopEngine(close = false): void {
  nativeCallSync('engine.stop', close)
}

/** An online game started or ended: the computer and analysis keep out of its way while it runs. */
export function setEngineBusy(online: boolean): void {
  nativeCallSync('engine.setBusy', online)
}

export function bestMove(
  moveList: unknown,
  level: EngineLevel,
  configured = '',
  options: BestMoveOptions = {},
): Promise<string> {
  return nativeCall<string>('engine.bestMove', moveList, level, configured, options)
}

export function resetEngine(): void {
  nativeCallSync('engine.reset')
}

/** Ends every engine process and waits for each to exit (`engines.close`). */
export async function closeEngines(): Promise<void> {
  await nativeCall('engines.close')
}
