import type { Middleware } from 'openapi-fetch'
import { rules } from './engine.ts'

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
 * The rules choose the detail (`records/lichess.rs`); the message is built here.
 */
export function lichessError(
  response: { status: number },
  error: unknown,
  endpoint: string,
): LichessError {
  const fields = rules<{ status: number; endpoint: string; detail: string }>(
    'lichessError',
    { status: response.status },
    error,
    endpoint,
  )
  return new LichessError(fields.status, fields.endpoint, fields.detail)
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
