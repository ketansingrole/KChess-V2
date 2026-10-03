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
/** Share rate-limit cooldown and foreground priority; never replay an ambiguous mutation. */
export const lichessFetch: typeof fetch = async (input, init) => {
  const original = new Request(input, init)
  const foreground =
    /\/api\/board\/game\/.*\/(move|resign|abort|takeback)\//.test(original.url) ||
    /\/api\/board\/game\/.*\/(resign|abort)$/.test(original.url)
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
      /\/api\/(stream\/event|board\/(game\/stream|seek)|games\/user|puzzle\/activity)/.test(
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
