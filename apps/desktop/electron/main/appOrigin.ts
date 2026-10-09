/** Custom schemes have opaque URL.origin values; compare protocol and authority explicitly. */
import { logDebug } from '../../../../core/src/services/logger'

export function isAppUrl(value: string, developmentUrl = process.env.KCHESS_NUXT_URL): boolean {
  try {
    const candidate = new URL(value)
    const expected = new URL(developmentUrl || 'kchess://app/')
    return (
      !candidate.username &&
      !candidate.password &&
      candidate.protocol === expected.protocol &&
      candidate.host === expected.host
    )
  } catch (cause) {
    logDebug('startup', 'Invalid URL for app origin check:', cause)
    return false
  }
}
export const APP_CSP =
  "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https://lichess.org https://image.lichess1.org; font-src 'self'; connect-src 'self' kchess://app data:; worker-src 'self' blob:; media-src 'self' blob:; object-src 'none'; base-uri 'none'; frame-src 'none'; frame-ancestors 'none'"

// Vosk's Emscripten bindings generate JS functions. This exception applies only to its
// build-extracted worker, which has no DOM or preload bridge, never to renderer documents.
export const VOICE_WORKER_CSP =
  "default-src 'none'; script-src 'self' 'unsafe-eval'; connect-src 'self' kchess://app; object-src 'none'"
