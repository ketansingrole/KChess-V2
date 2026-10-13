/**
 * The Node host's non-core methods: worker-isolated conveniences outside the
 * Rust contract tables (`settings`, `trustEnginePath`, `voiceHistoryDocument`,
 * `suspend`, `close`). Arity and the engine-path shape are pinned here so
 * `worker.ts` and its tests share one table instead of hand-coded counts.
 */

export const HOST_METHODS = [
  'settings',
  'trustEnginePath',
  'voiceHistoryDocument',
  'suspend',
  'close',
] as const

export type HostMethod = (typeof HOST_METHODS)[number]

const MAX_ENGINE_PATH = 4096

/** Throw on malformed non-core host arguments; core methods use `validateCoreArguments`. */
export function validateHostArguments(method: HostMethod, args: unknown[]): void {
  if (method === 'trustEnginePath') {
    const [path] = args
    if (args.length !== 1 || typeof path !== 'string' || !path || path.length > MAX_ENGINE_PATH)
      throw new Error('Invalid host arguments.')
    return
  }
  if (args.length !== 0) throw new Error('Invalid host arguments.')
}
