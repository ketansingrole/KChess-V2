import type { ChildProcessByStdio } from 'node:child_process'
import { timed } from './performance'
import { createInterface } from 'node:readline'
import type { Readable, Writable } from 'node:stream'

export class SearchCancelled extends Error {
  constructor() {
    super('Engine search cancelled.')
    this.name = 'SearchCancelled'
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

  constructor(
    private child: ChildProcessByStdio<Writable, Readable, Readable>,
    private deadline = 10_000,
  ) {
    const lines = createInterface({ input: child.stdout })
    lines.on('line', (raw) => {
      for (const listener of [...this.listeners]) listener(raw.trim())
    })
    child.stderr.resume()
    child.stdin.on('error', (error) => this.fail(error))
    child.on('error', (error) => this.fail(error))
    child.on('exit', () => {
      clearTimeout(this.termination)
      lines.close()
      this.fail(new Error('Stockfish stopped unexpectedly. Choose another engine or retry.'))
    })
    this.ready = this.initialize()
    // Initialization may fail before a consumer has reached its await.
    void this.ready.catch(() => {})
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
          ? setTimeout(
              () => this.fail(new Error('Stockfish timed out. Retry or choose another engine.')),
              timeout,
            )
          : undefined
      this.listeners.add(line)
      this.failures.add(fail)
      try {
        this.write(command)
      } catch (error) {
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
      } catch {
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
    if (this.failure) return
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
  }
}
