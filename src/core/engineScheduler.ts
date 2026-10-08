import { cpus } from 'node:os'
import { SearchCancelled } from './uci'

/** A total search budget; warm idle processes retain their small hash, not CPU reservations. */
const budget = Math.max(1, Math.min(4, cpus().length - 1))
let onBattery = (): boolean => false
export function configureEngineResources(battery: () => boolean): void {
  onBattery = battery
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
let active: Lease | undefined
const waiting: Waiting[] = []

function pump(): void {
  if (active) return
  waiting.sort((a, b) => b.priority - a.priority)
  const next = waiting.shift()
  if (!next) return
  next.signal?.removeEventListener('abort', next.abort)
  active = next
  let released = false
  next.resolve(() => {
    if (released) return
    released = true
    if (active === next) active = undefined
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
        const at = waiting.indexOf(entry)
        if (at >= 0) waiting.splice(at, 1)
        reject(new SearchCancelled())
      },
    }
    waiting.push(entry)
    signal?.addEventListener('abort', entry.abort, { once: true })
    if (active && priority > active.priority) active.stop()
    pump()
  })
}
export const searchThreads = (): number => (onBattery() ? Math.min(2, budget) : budget)

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
