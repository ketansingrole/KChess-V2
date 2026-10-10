import { nativeCall } from './nativeCore.ts'
import { logDebug } from './logger.ts'

/**
 * The Stockfish KChess downloads and owns, managed by the Rust core
 * (`crates/kchess-core/src/engine/managed.rs`). The core suspends the engines while it replaces the
 * executable and cancels a running install when it closes.
 */

export interface ManagedEngine {
  installed: boolean
  path: string
  /** Release tag recorded when it was downloaded, e.g. `sf_17.1`. */
  version?: string
}

export interface InstallResult extends ManagedEngine {
  version: string
  /** False when the installed engine already matched the latest release and nothing was downloaded. */
  updated: boolean
}

/**
 * Test-only locations (honoured only when the core's test switch is on): the directory holding the
 * engine and the release endpoint it is downloaded from.
 */
export interface ManagedLocation {
  dir?: string
  releaseUrl?: string
}

let requests = 0

/** The downloaded engine's state (`managedEngine.status`). */
export const managedEngine = (location?: ManagedLocation): Promise<ManagedEngine> =>
  nativeCall<ManagedEngine>('managedEngine.status', location ?? null)

/** Ends a request the caller no longer wants, when its signal aborts (`managedEngine.cancel`). */
function cancelOnAbort<T>(
  id: number,
  signal: AbortSignal | undefined,
  work: Promise<T>,
): Promise<T> {
  signal?.addEventListener(
    'abort',
    () => {
      void nativeCall('managedEngine.cancel', id).catch((cause: unknown) => {
        logDebug('managed-engine', 'Engine install cancel failed:', cause)
      })
    },
    { once: true },
  )
  return work
}

/** Remove the downloaded engine. Bundled and user-picked engines are never touched. */
export async function deleteManagedEngine(
  location?: ManagedLocation,
  signal?: AbortSignal,
): Promise<void> {
  const id = ++requests
  await cancelOnAbort(id, signal, nativeCall('managedEngine.delete', id, location ?? null))
}

/**
 * Install the latest official native build, verified against the digest GitHub publishes. Checks
 * the latest release first and downloads nothing when the installed engine is already that version.
 */
export function installManagedEngine(
  location?: ManagedLocation,
  signal?: AbortSignal,
): Promise<InstallResult> {
  const id = ++requests
  return cancelOnAbort(
    id,
    signal,
    nativeCall<InstallResult>('managedEngine.install', id, location ?? null),
  )
}
