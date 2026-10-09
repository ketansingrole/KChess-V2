import { Worker } from 'node:worker_threads'
import { join } from 'node:path'
import { CORE_METHODS } from '@kchess/core'
import { errorSummary, logWarn } from '@kchess/core/logger'
import type { CoreEvents } from '@kchess/core'
import type { NodeCoreOptions } from './platform'
import type { HostResponse, NodeCore } from './protocol'
export type { NodeCoreOptions } from './platform'
export type { NodeCore } from './protocol'

/** Each host owns a worker: profiles, caches, streams and engines never share globals. */
export async function createNodeCore(options: NodeCoreOptions = {}): Promise<NodeCore> {
  const worker = new Worker(join(import.meta.dirname, 'nodeWorker.js'), {
    workerData: options,
    stdout: true,
    stderr: true,
  })
  const finished = new Promise<void>((resolve) => worker.once('exit', () => resolve()))
  worker.stdout.pipe(process.stderr)
  worker.stderr.pipe(process.stderr)
  const pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (cause: Error) => void }
  >()
  const listeners = new Map<keyof CoreEvents, Set<(payload: never) => void>>()
  let nextId = 0,
    closed = false
  let closing: Promise<void> | undefined
  let readyResolve: () => void
  let readyReject: (cause: Error) => void
  const ready = new Promise<void>((resolve, reject) => {
    readyResolve = resolve
    readyReject = reject
  })
  const fail = (cause: Error): void => {
    closed = true
    readyReject(cause)
    for (const request of pending.values()) request.reject(cause)
    pending.clear()
    listeners.clear()
  }
  worker.on('error', fail)
  worker.on('exit', (code) => fail(new Error(`Node core host exited (${code}).`)))
  worker.on('message', (message: HostResponse) => {
    if ('ready' in message) readyResolve()
    else if ('startupError' in message) fail(new Error(message.startupError))
    else if ('event' in message) {
      if (closed) return
      for (const listener of listeners.get(message.event) ?? []) {
        try {
          listener(message.payload as never)
        } catch (cause) {
          logWarn('node-core', 'Event listener failed:', message.event, cause)
        }
      }
    } else {
      const request = pending.get(message.id)
      pending.delete(message.id)
      if (message.error) request?.reject(new Error(message.error))
      else request?.resolve(message.result)
    }
  })
  const timer = setTimeout(() => fail(new Error('Node core startup timed out.')), 30000)
  try {
    await ready
  } catch (cause) {
    await worker.terminate()
    throw cause
  } finally {
    clearTimeout(timer)
  }
  function call(method: string, args: unknown[]): Promise<unknown> {
    if (closed && method !== 'close') return Promise.reject(new Error('The KChess core is closed.'))
    const id = ++nextId
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject })
      try {
        worker.postMessage({ id, method, args })
      } catch (cause) {
        pending.delete(id)
        logWarn('node-core', 'Could not send core request:', method, errorSummary(cause))
        reject(cause instanceof Error ? cause : new Error('Could not send core request.'))
      }
    })
  }
  const api = Object.fromEntries(
    [...CORE_METHODS, 'settings', 'trustEnginePath', 'voiceHistoryDocument', 'suspend'].map(
      (method) => [method, (...args: unknown[]) => call(method, args)],
    ),
  )
  return Object.assign(api, {
    finished,
    on<K extends keyof CoreEvents>(event: K, listener: (payload: CoreEvents[K]) => void) {
      if (closed) throw new Error('The KChess core is closed.')
      let set = listeners.get(event)
      if (!set) listeners.set(event, (set = new Set()))
      set.add(listener as (payload: never) => void)
      return () => {
        set.delete(listener as (payload: never) => void)
      }
    },
    close() {
      if (closing) return closing
      if (closed) return finished
      closed = true
      listeners.clear()
      closing = (async () => {
        const backstop = setTimeout(() => {
          void worker.terminate()
        }, 10000)
        try {
          await call('close', [])
        } finally {
          clearTimeout(backstop)
          await worker.terminate()
        }
      })()
      return closing
    },
  }) as unknown as NodeCore
}
