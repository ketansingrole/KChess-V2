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
