import { bindCoreCallback, platform, scopedState } from './platform'
import { nativeRules, type NativeCoreHandle } from './native'
import { dbPath } from './db'
import { logDebug, logError, logInfo, logWarn } from './logger'
import { recordUsage } from './usage'
import type { UsageKind } from '../contracts/types'

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
  if (event === 'host:usage') {
    const usage = payload as {
      account: string
      category: UsageKind
      requests: number
      bytes: number
    }
    recordUsage(usage.account, usage.category, usage.requests, usage.bytes)
    return
  }
  for (const handler of state.handlers.get(event) ?? []) handler(payload)
}

function core(): NativeCoreHandle {
  if (state.core) return state.core
  const rules = nativeRules()
  if (!rules) throw new Error('The native core is not built (pnpm run build:native).')
  state.core = new rules.NativeCore(
    { dataDir: platform().dataDir, legacyDatabasePath: dbPath() },
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
