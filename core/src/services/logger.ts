import { format } from 'node:util'

/**
 * Structured main-process logging.
 *
 * Every entry carries a `[scope]` prefix so `logs/` can be grepped per
 * subsystem (`renderer`, `app-updates`, `notify`, …). The calls delegate to
 * `console.*` at call time, so `setupDiagnostics()` still captures them into
 * the bounded redacted log files, and tests spying on `console.warn` keep
 * working. Renderers must not import this module (see architecture
 * boundaries); renderer diagnostics go through IPC `reportRendererError`.
 *
 * Production conventions (see GUARDRAILS.md): include operation context
 * (`method`, `account`, `gameId`, `durationMs`, `failures`) with every
 * failure, truncate long payloads with `truncateForLog`, and summarize
 * errors with `errorSummary` so status/endpoint/code survive. Never log
 * tokens, passwords, or request bodies; `DiagnosticLog` redacts them.
 */
const TRUNCATE_MAX = 120

/** Bound a value for logs so one FEN/PGN/URL cannot flood the bounded files. */
export function truncateForLog(value: unknown, max = TRUNCATE_MAX): string {
  let text: string
  if (typeof value === 'string') text = value
  else {
    try {
      text = JSON.stringify(value) ?? String(value)
      // eslint-disable-next-line logging/no-silent-catch -- logger internals must not log; String() fallback preserves the entry.
    } catch {
      text = String(value)
    }
  }
  const single = text.replace(/\s+/g, ' ')
  return single.length > max
    ? `${single.slice(0, max)}…(${(text.length - max).toLocaleString()} more chars)`
    : single
}

/** Short `go`-style command name without the position payload. */
export function uciCommandName(command: string): string {
  const head = command.split(/\s+/)[0] ?? command
  const tail = command.split(' moves ')[1]
  const moveCount = tail ? tail.split(' ').length : 0
  const moves = / moves /.test(command) ? ` moves(${moveCount})` : ''
  return truncateForLog(`${head}${moves}`, 40)
}

/** True for cancellations that are routine control flow, not failures. */
export function isExpectedCancellation(cause: unknown): boolean {
  if (cause instanceof DOMException && cause.name === 'AbortError') return true
  if (typeof cause === 'object' && cause !== null) {
    const name = (cause as { name?: unknown }).name
    if (name === 'SearchCancelled') return true
    const message = (cause as { message?: unknown }).message
    if (typeof message === 'string' && /superseded|cancelled/i.test(message)) return true
  }
  return false
}

/** One-line error summary with status/endpoint/code when present. */
export function errorSummary(cause: unknown): string {
  if (typeof cause === 'string') return truncateForLog(cause, 300)
  if (!(cause instanceof Error)) return truncateForLog(cause, 300)
  const extra: string[] = []
  const status = (cause as unknown as Record<string, unknown>).status
  const endpoint = (cause as unknown as Record<string, unknown>).endpoint
  const code = (cause as unknown as Record<string, unknown>).code
  if (typeof status === 'number') extra.push(`status=${status}`)
  if (typeof endpoint === 'string' && endpoint)
    extra.push(`endpoint=${truncateForLog(endpoint, 80)}`)
  if ((typeof code === 'string' || typeof code === 'number') && code !== '')
    extra.push(`code=${String(code)}`)
  const head = `${cause.name}: ${cause.message}`
  return truncateForLog(extra.length ? `${head} (${extra.join(', ')})` : head, 300)
}
function write(
  level: 'log' | 'info' | 'warn' | 'error',
  scope: string,
  message: string,
  details: unknown[],
): void {
  const suffix = details.length ? ` ${format(...details)}` : ''
  const line = `[${scope}] ${message}${suffix}`
  if (level === 'warn') console.warn(line)
  else if (level === 'error') console.error(line)
  else if (level === 'info') console.info(line)
  else console.log(line)
}

/** Normal operational events (startup, successful loads). */
export function logInfo(scope: string, message: string, ...details: unknown[]): void {
  write('info', scope, message, details)
}

/** Recoverable failures (update check offline, cache write refused). */
export function logWarn(scope: string, message: string, ...details: unknown[]): void {
  write('warn', scope, message, details)
}

/** Failures needing attention (renderer crash, startup failure). */
export function logError(scope: string, message: string, ...details: unknown[]): void {
  write('error', scope, message, details)
}

/** Verbose diagnostics, captured via `console.log` so the log files keep it. */
export function logDebug(scope: string, message: string, ...details: unknown[]): void {
  write('log', scope, message, details)
}
