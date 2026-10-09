import { AsyncLocalStorage } from 'node:async_hooks'

/**
 * What the headless core needs from its host. The desktop shell supplies an Electron
 * implementation; other frontends supply their own. Core modules never import a frontend.
 */
export interface CorePlatform {
  /** Directory for KChess's databases and other local data; it must exist. */
  dataDir: string
  /** The bundled Stockfish UCI script, run with this process's Node runtime. */
  bundledEnginePath: string
  /** Extra environment for running the bundled engine with `process.execPath`. */
  nodeEnv?: Record<string, string>
  /** Where KChess keeps the Stockfish build it downloads; defaults to the data directory. */
  managedEngineDir?: string
  /** OS-backed encryption for stored Lichess tokens. */
  secrets: SecretStore
  /** Open a URL in the user's browser (Lichess sign-in). */
  openExternal(url: string): Promise<void>
  /** A database from an earlier KChess release, imported once into an empty profile. */
  legacyDatabasePath?: string
  /** Bring the frontend back to the foreground after the browser sign-in returns. */
  focus?(): void
  /** Background engine work yields while on battery power. */
  onBattery(): boolean
}

export interface SecretStore {
  /** False when only a plaintext fallback exists; tokens are then never stored. */
  available(): boolean
  /** Base64 ciphertext. */
  encrypt(plain: string): string
  decrypt(base64: string): string
}

interface PlatformScope {
  host: CorePlatform
  controller: AbortController
  closed: boolean
  states: Map<symbol, object>
}
let current: PlatformScope | undefined
const context = new AsyncLocalStorage<PlatformScope>()

export function setPlatform(host: CorePlatform): void {
  current = { host, controller: new AbortController(), closed: false, states: new Map() }
}
/** A fresh scope also works when another core creates a host inside an event callback. */
export function runWithPlatform<T>(host: CorePlatform, work: () => T): T {
  setPlatform(host)
  return context.run(current!, work)
}
function scope(): PlatformScope {
  const value = context.getStore() ?? current
  if (!value) throw new Error('The KChess core has not been configured with a platform.')
  return value
}
export function platform(): CorePlatform {
  const value = scope()
  if (value.closed) throw new Error('The KChess core is closed.')
  return value.host
}
/** Async work keeps its original host even after that host closes. */
export function bindPlatform<T extends unknown[], R>(work: (...args: T) => R): (...args: T) => R {
  const value = scope()
  return (...args) => context.run(value, () => work(...args))
}
/** Unconfigured low-level unit tests retain their caller's signal. */
export function coreSignal(): AbortSignal | undefined {
  return (context.getStore() ?? current)?.controller.signal
}
export function abortPlatform(): void {
  scope().controller.abort(new DOMException('Core closed.', 'AbortError'))
}
export function closePlatform(): void {
  scope().closed = true
}

/** A typed service state belongs to the current core, never to the module cache.
 * Low-level tests without a platform retain an independent fallback state.
 * Closed scopes keep their state until teardown finishes; platform() still rejects I/O.
 */
export function scopedState<T extends object>(create: () => T): T {
  const key = Symbol('core-service')
  let fallback: T | undefined
  const value = (): T => {
    const owner = context.getStore() ?? current
    if (!owner) return (fallback ??= create())
    let state = owner.states.get(key) as T | undefined
    if (!state) {
      state = create()
      owner.states.set(key, state)
    }
    return state
  }
  return new Proxy({} as T, {
    get(_target, property) {
      const state = value()
      const result = Reflect.get(state, property, state)
      return typeof result === 'function' ? result.bind(state) : result
    },
    set(_target, property, next) {
      return Reflect.set(value(), property, next)
    },
  })
}

/** Bind EventEmitter/stream callbacks, which otherwise run in their emitter's context. */
export function bindCoreCallback<T extends unknown[], R>(
  work: (...args: T) => R,
): (...args: T) => R {
  const owner = context.getStore() ?? current
  return owner ? (...args) => context.run(owner, () => work(...args)) : work
}
