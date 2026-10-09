import { scopedState } from './platform'
import { cpus } from 'node:os'
import { SearchCancelled } from './uci'

/** A total search budget; warm idle processes retain their small hash, not CPU reservations. */
const budget = Math.max(1, Math.min(4, cpus().length - 1))

export function configureEngineResources(battery: () => boolean): void {
  serviceState.onBattery = battery
}
interface Lease {
  priority: number
  stop: () => void
}
interface Waiting extends Lease {
  signal?: AbortSignal
  resolve: (release: () => void) => void
  reject: (error: Error) => void
  abort: () => void
}

function pump(): void {
  if (serviceState.active) return
  serviceState.waiting.sort((a, b) => b.priority - a.priority)
  const next = serviceState.waiting.shift()
  if (!next) return
  next.signal?.removeEventListener('abort', next.abort)
  serviceState.active = next
  let released = false
  next.resolve(() => {
    if (released) return
    released = true
    if (serviceState.active === next) serviceState.active = undefined
    pump()
  })
}

/** Computer moves may interrupt analysis/review. Every acquisition must release in finally. */
export function acquireEngine(
  priority: number,
  stop: () => void,
  signal?: AbortSignal,
): Promise<() => void> {
  if (signal?.aborted) return Promise.reject(new SearchCancelled())
  return new Promise((resolve, reject) => {
    const entry: Waiting = {
      priority,
      stop,
      signal,
      resolve,
      reject,
      abort: () => {
        const at = serviceState.waiting.indexOf(entry)
        if (at >= 0) serviceState.waiting.splice(at, 1)
        reject(new SearchCancelled())
      },
    }
    serviceState.waiting.push(entry)
    signal?.addEventListener('abort', entry.abort, { once: true })
    if (serviceState.active && priority > serviceState.active.priority) serviceState.active.stop()
    pump()
  })
}
export const searchThreads = (): number => (serviceState.onBattery() ? Math.min(2, budget) : budget)

/** Configure and search only inside this lease; all exits release the shared scheduler. */
export async function withEngineLease<T>(
  priority: number,
  stop: () => void,
  signal: AbortSignal,
  work: () => Promise<T>,
): Promise<T> {
  const release = await acquireEngine(priority, stop, signal)
  try {
    signal.throwIfAborted()
    return await work()
  } finally {
    release()
  }
}

const serviceState = scopedState(() => ({
  onBattery: (): boolean => false,
  active: undefined as Lease | undefined,
  waiting: [] as Waiting[],
}))
