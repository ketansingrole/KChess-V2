import { bindCoreCallback, platform, scopedState } from './platform.ts'
import { nativeRules, type NativeCoreHandle } from './native.ts'
import { logDebug, logError, logInfo, logWarn } from './logger.ts'

/**
 * The Rust core's services (`crates/kchess-core`), one instance per core scope, while the core
 * migrates to Rust (`RUST_MIGRATION.md`). Its logs go to the core logger; its events reach
 * the handlers registered with `onNativeEvent`, and `host:*` events ask TypeScript services
 * that have not moved yet to act.
 */
type Handler = (payload: unknown) => void

const state = scopedState(() => ({
  core: undefined as NativeCoreHandle | undefined,
  handlers: new Map<string, Set<Handler>>(),
}))

const LOG = { debug: logDebug, info: logInfo, warn: logWarn, error: logError } as const

function handle(event: string, payload: unknown): void {
  if (event === 'host:request') {
    void answer(payload as { id: number; kind: string; payload: unknown })
    return
  }
  for (const handler of state.handlers.get(event) ?? []) handler(payload)
}

/** What the Rust core may ask of the host platform (`crates/kchess-core/src/capabilities.rs`). */
async function capability(kind: string, payload: unknown): Promise<unknown> {
  const host = platform()
  switch (kind) {
    case 'secrets.available':
      return host.secrets.available()
    case 'secrets.encrypt':
      return host.secrets.encrypt(String(payload))
    case 'secrets.decrypt':
      return host.secrets.decrypt(String(payload))
    case 'openExternal':
      return host.openExternal(String(payload))
    case 'focus':
      return host.focus?.()
    case 'onBattery':
      return host.onBattery()
    default:
      throw new Error(`Unknown host capability ${kind}.`)
  }
}

/** Answer a `host:request`; the core bounds how long it waits. Never logs the payload. */
async function answer(request: { id: number; kind: string; payload: unknown }): Promise<void> {
  let reply: { value: unknown } | { error: string }
  try {
    reply = { value: (await capability(request.kind, request.payload)) ?? null }
  } catch (cause) {
    logWarn('native-core', 'Host capability failed:', request.kind, cause)
    reply = { error: cause instanceof Error ? cause.message : String(cause) }
  }
  try {
    await nativeCall('host.reply', request.id, reply)
  } catch (cause) {
    logDebug('native-core', 'Host reply was not delivered:', request.kind, cause)
  }
}

function core(): NativeCoreHandle {
  if (state.core) return state.core
  const rules = nativeRules()
  if (!rules) throw new Error('The native core is not built (pnpm run build:native).')
  const host = platform()
  state.core = new rules.NativeCore(
    {
      dataDir: host.dataDir,
      legacyDatabasePath: host.legacyDatabasePath,
      bundledEnginePath: host.bundledEnginePath,
      nodePath: process.execPath,
      nodeEnv: host.nodeEnv,
      managedEngineDir: host.managedEngineDir,
    },
    bindCoreCallback((json: string) => {
      const line = JSON.parse(json) as { level: keyof typeof LOG; scope: string; message: string }
      LOG[line.level](line.scope, line.message)
    }),
    bindCoreCallback((json: string) => {
      const { event, payload } = JSON.parse(json) as { event: string; payload: unknown }
      handle(event, payload)
    }),
  )
  return state.core
}

/** Call a Rust core method; rejects with its message (an `AbortError` when cancelled). */
export async function nativeCall<T>(method: string, ...args: unknown[]): Promise<T> {
  try {
    return JSON.parse(await core().call(method, JSON.stringify(args))) as T
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause)
    if (message.startsWith('AbortError: '))
      throw new DOMException(message.slice('AbortError: '.length), 'AbortError')
    throw new Error(message, { cause })
  }
}

/** Call a synchronous Rust core method (storage); throws its message. */
export function nativeCallSync<T>(method: string, ...args: unknown[]): T {
  return JSON.parse(core().callSync(method, JSON.stringify(args))) as T
}

/** Receive a Rust core event until the returned function is called. */
export function onNativeEvent<T>(event: string, handler: (payload: T) => void): () => void {
  const handlers = state.handlers.get(event) ?? new Set()
  state.handlers.set(event, handlers)
  handlers.add(handler as Handler)
  return () => handlers.delete(handler as Handler)
}

/** Cancel the Rust core's work and release its files. */
export async function closeNativeCore(): Promise<void> {
  const previous = state.core
  state.core = undefined
  state.handlers.clear()
  await previous?.close()
}
