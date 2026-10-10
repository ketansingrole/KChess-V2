import type { CoreApi, KChessCore, CoreEvents } from '@kchess/native'

export type NodeMethods = CoreApi &
  Pick<KChessCore, 'settings' | 'trustEnginePath' | 'voiceHistoryDocument' | 'suspend'>
export type NodeApi = {
  [K in keyof NodeMethods]: NodeMethods[K] extends (...args: infer A) => infer R
    ? (...args: A) => Promise<Awaited<R>>
    : never
}
export interface NodeCore extends NodeApi {
  /** Resolves when the host worker exits, including unexpected termination. */
  readonly finished: Promise<void>
  on<K extends keyof CoreEvents>(event: K, listener: (payload: CoreEvents[K]) => void): () => void
  close(): Promise<void>
}
export type HostRequest = { id: number; method: keyof NodeMethods | 'close'; args: unknown[] }
export type HostResponse =
  | { ready: true }
  | { event: keyof CoreEvents; payload: unknown }
  | { id: number; result?: unknown; error?: string }
  | { startupError: string }
