import { scopedState, bindCoreCallback } from './platform'
import type { ChildProcessByStdio } from 'node:child_process'
import { timed } from './performance'
import { errorSummary, isExpectedCancellation, logDebug, logWarn, uciCommandName } from './logger'
import { createInterface } from 'node:readline'
import type { Readable, Writable } from 'node:stream'

export class SearchCancelled extends Error {
  constructor() {
    super('Engine search cancelled.')
    this.name = 'SearchCancelled'
  }
}

export function assertEngineAvailable(): void {
  if (serviceState.maintenance) throw new Error('Stockfish is being updated. Retry shortly.')
}
/** Prevent new processes while all owners release the executable being replaced. */
export async function withEngineMaintenance<T>(
  action: () => Promise<T>,
  stopOwners: () => void = () => {},
): Promise<T> {
  assertEngineAvailable()
  serviceState.maintenance = true
  let deadline: ReturnType<typeof setTimeout> | undefined
  try {
    stopOwners()
    await Promise.race([
      Promise.all(
        [...serviceState.controllers].map((controller) => {
          controller.close()
          return controller.closed
        }),
      ),
      new Promise<never>((_resolve, reject) => {
        deadline = setTimeout(
          () => reject(new Error('Stockfish did not exit. The previous engine was retained.')),
          4000,
        )
      }),
    ])
    return await action()
  } finally {
    clearTimeout(deadline)
    serviceState.maintenance = false
  }
}

/** Wait for termination before a worker or host exits, including the SIGKILL backstop. */
export async function closeEngines(): Promise<void> {
  let deadline: ReturnType<typeof setTimeout> | undefined
  try {
    await Promise.race([
      Promise.all(
        [...serviceState.controllers].map((controller) => {
          controller.close()
          return controller.closed
        }),
      ),
      new Promise<never>((_resolve, reject) => {
        deadline = setTimeout(
          () => reject(new Error('Stockfish did not exit during shutdown.')),
          4000,
        )
      }),
    ])
  } finally {
    clearTimeout(deadline)
  }
}

/** Owns pipes, deadlines and termination. Consumers own positions and interpretation of info. */
export class UciController {
  private listeners = new Set<(line: string) => void>()
  private failures = new Set<(error: Error) => void>()
  private failure?: Error
  private searching = false
  private stopSearch?: () => void
  private termination?: ReturnType<typeof setTimeout>
  readonly ready: Promise<void>
  readonly closed: Promise<void>

  constructor(
    private child: ChildProcessByStdio<Writable, Readable, Readable>,
    private deadline = 10_000,
  ) {
    serviceState.controllers.add(this)
    this.closed = new Promise((resolve) => {
      const ended = (): void => {
        serviceState.controllers.delete(this)
        resolve()
      }
      child.once('exit', bindCoreCallback(ended))
      child.once('error', bindCoreCallback(ended))
    })
    const lines = createInterface({ input: child.stdout })
    lines.on(
      'line',
      bindCoreCallback((raw) => {
        const line = raw.trim()
        if (!line) return
        // One listener is the common case (a single search); snapshot only when
        // several listen, so per-line `info` traffic avoids a Set copy and a
        // trim per listener. Direct Set iteration is safe here: listeners only
        // ever remove themselves after matching.
        if (this.listeners.size === 1) {
          for (const listener of this.listeners) listener(line)
          return
        }
        for (const listener of [...this.listeners]) listener(line)
      }),
    )
    // Keep the first bytes of stderr for failure diagnostics; Stockfish is
    // quiet on success, so this stays empty in the common case.
    let stderrSnippet = ''
    child.stderr.on(
      'data',
      bindCoreCallback((chunk: Buffer) => {
        if (stderrSnippet.length < 2048) {
          stderrSnippet += chunk.toString('utf8', 0, 2048 - stderrSnippet.length)
        }
      }),
    )
    child.stderr.resume()
    child.stdin.on(
      'error',
      bindCoreCallback((error) => this.fail(error)),
    )
    child.on(
      'error',
      bindCoreCallback((error) => this.fail(error)),
    )
    child.on(
      'exit',
      bindCoreCallback((code, signal) => {
        clearTimeout(this.termination)
        lines.close()
        if (this.failure) {
          logDebug('uci', 'Engine process exited:', `code=${code}`, `signal=${signal ?? 'none'}`)
          return
        }
        logWarn(
          'uci',
          'Engine process exited unexpectedly:',
          `code=${code}`,
          `signal=${signal ?? 'none'}`,
          stderrSnippet
            ? `stderr=${stderrSnippet.replace(/\s+/g, ' ').slice(0, 200)}`
            : 'stderr=empty',
        )
        this.fail(new Error('Stockfish stopped unexpectedly. Choose another engine or retry.'))
      }),
    )
    this.ready = this.initialize()
    // Initialization may fail before a consumer has reached its await.
    void this.ready.catch((error: unknown) => {
      logDebug('uci', 'Engine init deferred:', error)
    })
  }

  get failed(): Error | undefined {
    return this.failure
  }
  write(command: string): void {
    if (this.failure) throw this.failure
    if (/[\r\n]/.test(command)) throw new Error('Invalid engine command.')
    this.child.stdin.write(`${command}\n`)
  }

  private async initialize(): Promise<void> {
    const complete = timed('engine.startup')
    try {
      await this.command('uci', (line) => line === 'uciok', this.deadline)
      await this.sync()
    } finally {
      complete()
    }
  }

  sync(): Promise<string> {
    return this.command('isready', (line) => line === 'readyok', this.deadline)
  }

  command(
    command: string,
    matches: (line: string) => boolean,
    timeout: number,
    onLine?: (line: string) => void,
  ): Promise<string> {
    if (this.failure) return Promise.reject(this.failure)
    return new Promise((resolve, reject) => {
      const clean = (): void => {
        clearTimeout(timer)
        this.listeners.delete(line)
        this.failures.delete(fail)
      }
      const fail = (error: Error): void => {
        clean()
        reject(error)
      }
      const line = (value: string): void => {
        onLine?.(value)
        if (matches(value)) {
          clean()
          resolve(value)
        }
      }
      const timer =
        timeout > 0
          ? setTimeout(() => {
              logWarn(
                'uci',
                'Engine command timed out:',
                uciCommandName(command),
                `timeoutMs=${timeout}`,
              )
              this.fail(new Error('Stockfish timed out. Retry or choose another engine.'))
            }, timeout)
          : undefined
      this.listeners.add(line)
      this.failures.add(fail)
      try {
        this.write(command)
      } catch (error) {
        logWarn('uci', 'Engine command write failed:', uciCommandName(command), errorSummary(error))
        fail(error instanceof Error ? error : new Error(String(error)))
      }
    })
  }

  async search(
    command: string,
    onLine: (line: string) => void,
    timeout: number,
    signal?: AbortSignal,
  ): Promise<string> {
    signal?.throwIfAborted()
    if (this.searching) throw new Error('An engine search is already active.')
    this.searching = true
    const complete = timed('engine.search')
    let cancelled = false
    let stopTiming: (() => void) | undefined
    let stopping: ReturnType<typeof setTimeout> | undefined
    const stop = (): void => {
      if (cancelled) return
      cancelled = true
      stopTiming = timed('engine.stop')
      try {
        this.write('stop')
      } catch (cause) {
        logDebug('uci', 'Engine stop failed:', cause)
        /* failure rejects the waiter */
      }
      stopping = setTimeout(
        () => this.fail(new Error('Stockfish did not stop. Retry the search.')),
        2_000,
      )
    }
    this.stopSearch = stop
    signal?.addEventListener('abort', stop, { once: true })
    try {
      const result = await this.command(
        command,
        (line) => line.startsWith('bestmove ') || line.startsWith('Nodes searched'),
        timeout,
        onLine,
      )
      if (cancelled) throw new SearchCancelled()
      return result
    } finally {
      clearTimeout(stopping)
      signal?.removeEventListener('abort', stop)
      this.stopSearch = undefined
      this.searching = false
      complete()
      stopTiming?.()
    }
  }

  stop(): void {
    this.stopSearch?.()
  }
  private fail(error: Error): void {
    if (this.failure) {
      logDebug('uci', 'Engine failure already reported:', errorSummary(error))
      return
    }
    if (isExpectedCancellation(error)) {
      logDebug('uci', 'Engine stopped:', errorSummary(error))
    } else {
      logWarn('uci', 'Engine failed:', errorSummary(error))
    }
    this.failure = error
    for (const listener of [...this.failures]) listener(error)
    this.listeners.clear()
    this.failures.clear()
    this.child.kill()
    this.termination = setTimeout(() => this.child.kill('SIGKILL'), 1_000)
    this.termination.unref()
  }
  close(): void {
    this.fail(new SearchCancelled())
    clearEngineOptions(this)
  }
}

/**
 * Engine options already confirmed on each process. `setoption` (notably `Hash`
 * and `Threads`) can clear the transposition table or reallocate threads, and
 * every redundant write costs an `isready` round-trip, so only changed options
 * are sent. Review pays this per position (100+ times per game) and analysis
 * per board step; computer moves reuse the warm process with the same level.
 */

/** Forget what was confirmed (engine replaced or closed; a new process starts empty). */
export function clearEngineOptions(target: UciController): void {
  serviceState.confirmedOptions.delete(target)
}

/**
 * Write only options whose value changed since the last successful sync, then
 * `isready` once when anything changed. Unchanged options need no round-trip:
 * the engine is idle between searches (`bestmove` already returned).
 */
export async function ensureEngineOptions(
  target: UciController,
  options: Record<string, string>,
): Promise<void> {
  let known = serviceState.confirmedOptions.get(target)
  if (!known) {
    known = new Map<string, string>()
    serviceState.confirmedOptions.set(target, known)
  }
  const pending: [string, string][] = []
  for (const [name, value] of Object.entries(options)) {
    if (known.get(name) !== value) pending.push([name, value])
  }
  if (!pending.length) return
  for (const [name, value] of pending) target.write(`setoption name ${name} value ${value}`)
  try {
    await target.sync()
  } catch (cause) {
    // A failed engine is recreated; its replacement must configure from scratch.
    serviceState.confirmedOptions.delete(target)
    throw cause
  }
  for (const [name, value] of pending) known.set(name, value)
}

const serviceState = scopedState(() => ({
  controllers: new Set<UciController>(),
  maintenance: false,
  confirmedOptions: new WeakMap<UciController, Map<string, string>>(),
}))
