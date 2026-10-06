import type { Middleware } from 'openapi-fetch'

/** Error thrown for failed Lichess API calls. The message is UI-safe. */
export class LichessError extends Error {
  status: number
  endpoint: string
  constructor(status: number, endpoint: string, detail?: string) {
    super(
      detail
        ? `Lichess ${status} from ${endpoint}: ${detail}`
        : `Lichess ${status} from ${endpoint}`,
    )
    this.name = 'LichessError'
    this.status = status
    this.endpoint = endpoint
  }
}

/**
 * Build a UI-safe error for a failed Lichess API call.
 * Lichess serves its website HTML 404 page (instead of API JSON) when a
 * username-based route doesn't resolve, so raw bodies must never be shown.
 */
export function lichessError(
  response: { status: number },
  error: unknown,
  endpoint: string,
): LichessError {
  if (typeof error === 'string' && error.trimStart().startsWith('<')) {
    if (response.status === 404) return new LichessError(404, endpoint, 'not found')
    return new LichessError(response.status, endpoint, 'unexpected page response')
  }
  const detail = error === undefined || error === null ? '' : JSON.stringify(error).slice(0, 250)
  return new LichessError(response.status, endpoint, detail)
}

/**
 * openapi-fetch middleware: every non-2xx response becomes a `LichessError`
 * (named after the request, e.g. `GET /api/user/magnus`), so call sites only
 * deal with successful data.
 */
export const throwLichessErrors: Middleware = {
  async onResponse({ request, response }) {
    if (response.ok) return undefined
    const text = await response.clone().text()
    let body: unknown = text || undefined
    try {
      body = text ? JSON.parse(text) : undefined
    } catch (cause) {
      // Not JSON (Lichess serves its HTML 404 page for unknown users); `lichessError` handles that.
      console.warn('[lichessError] Non-JSON error body, using raw text', cause)
    }
    throw lichessError(response, body, `${request.method} ${new URL(request.url).pathname}`)
  },
}
