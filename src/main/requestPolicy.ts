import { setTimeout as sleep } from 'node:timers/promises'
import { meteredFetch } from './usage'

let cooldownUntil = 0
let active = 0
const waiting: { priority: number; start: () => void }[] = []
function pump(): void {
  waiting.sort((a, b) => b.priority - a.priority)
  while (active < 2 && waiting.length) {
    active++
    waiting.shift()!.start()
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
let bulkActive = false
const bulkWaiting: (() => void)[] = []

function acquireBulk(signal: AbortSignal): Promise<() => void> {
  return new Promise((resolve, reject) => {
    let released = false
    const release = (): void => {
      if (released) return
      released = true
      const next = bulkWaiting.shift()
      if (next) next()
      else bulkActive = false
    }
    const start = (): void => {
      signal.removeEventListener('abort', abort)
      resolve(release)
    }
    const abort = (): void => {
      const at = bulkWaiting.indexOf(start)
      if (at >= 0) bulkWaiting.splice(at, 1)
      reject(signal.reason)
    }
    if (signal.aborted) {
      reject(signal.reason)
      return
    }
    if (!bulkActive) {
      bulkActive = true
      resolve(release)
      return
    }
    signal.addEventListener('abort', abort, { once: true })
    bulkWaiting.push(start)
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
  const original = new Request(input, init)
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
  if (waiting.length >= 500)
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
      const at = waiting.indexOf(entry)
      if (at >= 0) waiting.splice(at, 1)
      reject(original.signal.reason)
    }
    if (original.signal.aborted) {
      reject(original.signal.reason)
      return
    }
    original.signal.addEventListener('abort', abort, { once: true })
    waiting.push(entry)
    pump()
  })
  try {
    if (cooldownUntil > Date.now())
      await sleep(cooldownUntil - Date.now(), undefined, { signal: original.signal })
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
      cooldownUntil =
        Date.now() +
        (Number.isFinite(seconds) && seconds > 0 ? Math.min(300, seconds) * 1000 : 60_000)
    }
    return response
  } finally {
    active--
    pump()
  }
}
