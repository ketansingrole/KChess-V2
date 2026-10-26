import { CORE_METHODS, type CoreEvents } from '@kchess/contracts/core'
import type { CoreApi, CoreSettings, Settings } from '@kchess/contracts/types'
import {
  closeNativeCore,
  nativeCall,
  nativeConnection,
  nativeFocus,
  nativeReady,
  onNativeEvents,
  openNativeCore,
} from './nativeCore'
import { bindPlatform, closePlatform, runWithPlatform, type CorePlatform } from './platform'
import { logWarn } from './logger'
// Installs the domain rules on the native module; any core user needs them (`rules.ts`).
import './rules'

/**
 * The headless KChess core, as a frontend holds it. The Rust core answers every method
 * (`crates/kchess-core/src/facade.rs`): this module only forwards each call and each event, so it
 * makes no decisions of its own.
 */

type Listener<K extends keyof CoreEvents> = (payload: CoreEvents[K]) => void

/** The headless KChess services behind any frontend. */
export interface KChessCore extends CoreApi {
  on<K extends keyof CoreEvents>(event: K, listener: Listener<K>): () => void
  /** The saved settings, for frontend features outside the core. */
  settings(): Promise<CoreSettings>
  /** Desktop adapter compatibility; headless clients never receive these preferences. */
  desktopSettings(): Promise<Settings>
  saveDesktopSettings(settings: Settings): Promise<Settings>
  /** Allow an engine executable the user picked themselves to be saved in settings. */
  trustEnginePath(path: string): Promise<void>
  /** The voice log as a JSON document, for the frontend to save. */
  voiceHistoryDocument(): Promise<string>
  /**
   * Settles once the core can answer. With the shared core (`docs/shared-core.md`) it rejects
   * when another KChess release has the profile open; the message names the app to update.
   */
  ready(): Promise<void>
  /** This app's window gained or lost focus: the window focused last raises system alerts. */
  focus(focused: boolean): void
  /** This app's connection on the core: `origin` of its own writes in `profile:changed`. */
  connection(): number
  /** Stop live work nothing can answer without a frontend; services stay usable. */
  suspend(): void
  /** Stop everything and close the databases. */
  close(): Promise<void>
}

/** Facade methods outside `CoreApi`, answered asynchronously. */
const EXTRA_METHODS = ['settings', 'desktopSettings', 'saveDesktopSettings'] as const

/**
 * Configure an independent core for a host. Distinct profiles can coexist in one runtime; a second
 * live core for the same profile throws. Await `close()` before reopening that profile.
 */
export function createKChessCore(platform: CorePlatform): KChessCore {
  return runWithPlatform(platform, createCore)
}

function createCore(): KChessCore {
  const inScope = bindPlatform(<T>(work: () => T): T => work())
  // Opening claims the profile, so a profile in use is reported here.
  openNativeCore()
  const listeners = new Map<keyof CoreEvents, Set<(payload: never) => void>>()
  const stopEvents = onNativeEvents((event, payload) => {
    for (const listener of listeners.get(event as keyof CoreEvents) ?? []) {
      try {
        ;(listener as (payload: unknown) => void)(payload)
      } catch (cause) {
        logWarn('core', 'Event listener failed:', event, cause)
      }
    }
  })

  const forwarders: Record<string, (...args: unknown[]) => unknown> = {}
  for (const method of [...CORE_METHODS, ...EXTRA_METHODS])
    forwarders[method] = (...args) => inScope(() => nativeCall(method, ...args))

  let closing: Promise<void> | undefined
  const close = (): Promise<void> => {
    closing ??= (async () => {
      await inScope(() => nativeCall<unknown>('close'))
      stopEvents()
      listeners.clear()
      inScope(() => closePlatform())
      await inScope(() => closeNativeCore())
    })()
    return closing
  }

  return Object.assign(forwarders, {
    on(event: keyof CoreEvents, listener: (payload: never) => void) {
      let set = listeners.get(event)
      if (!set) listeners.set(event, (set = new Set()))
      set.add(listener)
      return () => void set.delete(listener)
    },
    trustEnginePath: async (path: string) => {
      await inScope(() => nativeCall<unknown>('trustEnginePath', path))
    },
    voiceHistoryDocument: () => inScope(() => nativeCall<string>('voiceHistoryDocument')),
    ready: () => inScope(() => nativeReady()),
    focus: (focused: boolean) => inScope(() => nativeFocus(focused)),
    connection: () => inScope(() => nativeConnection()),
    suspend() {
      void inScope(() => nativeCall<unknown>('suspend')).catch((cause: unknown) =>
        logWarn('core', 'Core suspend failed:', cause),
      )
    },
    close,
  }) as unknown as KChessCore
}
