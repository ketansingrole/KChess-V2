import { scopedState, coreSignal } from './platform'
import { setTimeout as sleep } from 'node:timers/promises'
import { logWarn } from './logger'
import { meteredFetch } from './usage'

function pump(): void {
  serviceState.waiting.sort((a, b) => b.priority - a.priority)
  while (serviceState.active < 2 && serviceState.waiting.length) {
    serviceState.active++
    serviceState.waiting.shift()!.start()
  }
}
/**
 * Game exports stream for up to minutes, long after their headers free a slot above.
 * Lichess asks for one export at a time, so they hold a lane of their own until the
 * body is read, cancelled or fails; interactive requests never queue behind them.
 */
const BULK_EXPORT = /\/api\/games\/(user|export)\//
/** Backstop in case a caller drops a body without reading or cancelling it. */
const BULK_HOLD_MS = 10 * 60_000

function acquireBulk(signal: AbortSignal): Promise<() => void> {
  return new Promise((resolve, reject) => {
    let released = false
    const release = (): void => {
      if (released) return
      released = true
      const next = serviceState.bulkWaiting.shift()
      if (next) next()
      else serviceState.bulkActive = false
    }
    const start = (): void => {
      signal.removeEventListener('abort', abort)
      resolve(release)
    }
    const abort = (): void => {
      const at = serviceState.bulkWaiting.indexOf(start)
      if (at >= 0) serviceState.bulkWaiting.splice(at, 1)
      reject(signal.reason)
    }
    if (signal.aborted) {
      reject(signal.reason)
      return
    }
    if (!serviceState.bulkActive) {
      serviceState.bulkActive = true
      resolve(release)
      return
    }
    signal.addEventListener('abort', abort, { once: true })
    serviceState.bulkWaiting.push(start)
  })
}

/** Pass the body through, calling `release` once it ends, errors or is cancelled. */
function releaseWithBody(response: Response, release: () => void): Response {
  if (!response.ok || !response.body) {
    release()
    return response
  }
  const backstop = setTimeout(release, BULK_HOLD_MS)
  backstop.unref()
  const done = (): void => {
    clearTimeout(backstop)
    release()
  }
  const reader = response.body.getReader()
  const body = new ReadableStream<Uint8Array>({
    async pull(controller) {
      try {
        const { done: ended, value } = await reader.read()
        if (ended) {
          done()
          controller.close()
        } else controller.enqueue(value)
      } catch (cause) {
        logWarn('request-policy', 'Bulk export body failed:', cause)
        done()
        controller.error(cause)
      }
    },
    cancel(reason) {
      done()
      return reader.cancel(reason)
    },
  })
  const wrapped = new Response(body, {
    status: response.status,
    statusText: response.statusText,
    headers: response.headers,
  })
  Object.defineProperty(wrapped, 'url', { value: response.url })
  return wrapped
}

/** Share rate-limit cooldown and foreground priority; never replay an ambiguous mutation. */
export const lichessFetch: typeof fetch = async (input, init) => {
  const request = new Request(input, init)
  const lifetime = coreSignal()
  const original = lifetime
    ? new Request(request, { signal: AbortSignal.any([request.signal, lifetime]) })
    : request
  original.signal.throwIfAborted()
  if (BULK_EXPORT.test(original.url)) {
    const release = await acquireBulk(original.signal)
    let response: Response
    try {
      response = await admitted(original)
    } catch (cause) {
      release()
      throw cause
    }
    return releaseWithBody(response, release)
  }
  return admitted(original)
}

async function admitted(original: Request): Promise<Response> {
  const foreground =
    /\/api\/board\/game\/.*\/(move|resign|abort|takeback|draw)\//.test(original.url) ||
    /\/api\/board\/game\/.*\/(resign|abort|claim-victory|claim-draw|berserk)$/.test(original.url)
  if (serviceState.waiting.length >= 500)
    throw new Error('Too many Lichess requests are pending. Retry shortly.')
  await new Promise<void>((resolve, reject) => {
    const entry = {
      priority: foreground ? 2 : 1,
      start: () => {
        original.signal.removeEventListener('abort', abort)
        resolve()
      },
    }
    const abort = (): void => {
      const at = serviceState.waiting.indexOf(entry)
      if (at >= 0) serviceState.waiting.splice(at, 1)
      reject(original.signal.reason)
    }
    if (original.signal.aborted) {
      reject(original.signal.reason)
      return
    }
    original.signal.addEventListener('abort', abort, { once: true })
    serviceState.waiting.push(entry)
    pump()
  })
  try {
    if (serviceState.cooldownUntil > Date.now())
      await sleep(serviceState.cooldownUntil - Date.now(), undefined, { signal: original.signal })
    const controller = new AbortController()
    const timer = setTimeout(
      () => controller.abort(new Error('Lichess request timed out. Retry or reconnect.')),
      30_000,
    )
    timer.unref()
    const signal = AbortSignal.any([original.signal, controller.signal])
    let response: Response
    try {
      response = await meteredFetch(new Request(original, { signal }))
    } catch (cause) {
      clearTimeout(timer)
      throw cause
    }
    // Stream heartbeats own their lifetime deadline; a header deadline must not kill a live game.
    if (
      /ndjson/.test(response.headers.get('content-type') ?? '') ||
      /\/api\/(stream\/|board\/(game\/stream|seek)|games\/user|puzzle\/activity|tv\/[^/]+\/feed)/.test(
        original.url,
      )
    )
      clearTimeout(timer)
    if (response.status === 429) {
      const retry = response.headers.get('retry-after')
      const seconds = Number(retry)
      serviceState.cooldownUntil =
        Date.now() +
        (Number.isFinite(seconds) && seconds > 0 ? Math.min(300, seconds) * 1000 : 60_000)
      try {
        const path = new URL(original.url).pathname
        logWarn(
          'request-policy',
          'Lichess rate limited:',
          path.slice(0, 80),
          `retryAfter=${retry ?? 'none'}`,
          `cooldownMs=${serviceState.cooldownUntil - Date.now()}`,
        )
      } catch {
        logWarn('request-policy', 'Lichess rate limited:', `retryAfter=${retry ?? 'none'}`)
      }
    }
    return response
  } finally {
    serviceState.active--
    pump()
  }
}

const serviceState = scopedState(() => ({
  cooldownUntil: 0,
  active: 0,
  waiting: [] as { priority: number; start: () => void }[],
  bulkActive: false,
  bulkWaiting: [] as (() => void)[],
}))
