import { spawn, execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { copyFileSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync } from 'node:fs'
import { connect, type Socket } from 'node:net'
import { homedir } from 'node:os'
import { basename, join } from 'node:path'
import { logDebug, logError, logWarn } from './logger.ts'
import type { NativeCoreHandle } from './native.ts'

/**
 * A client of the profile's shared core service (`docs/shared-core.md`,
 * `crates/kchess-service`): the same calls and events as the core embedded through N-API, over
 * the service's local socket. The service runs from this release's copy of the bundled binary;
 * it is started when no core runs for the profile, and started again if it stops.
 */

/** What this client can do for the core when it asks (`host:request`). */
const CAPABILITIES = ['browser', 'focus', 'legacySecrets'] as const
const START_DEADLINE_MS = 5000
const RETRY_MS = 50
const MAX_FRAME = 16 * 1024 * 1024
const STOPPED = 'The KChess core stopped.'

interface CoreInfo {
  pid: number
  socket: string
  release: string
  protocol: string
}

interface ClientSummary {
  kind: string
  release: string
}

type Frame =
  | { type: 'welcome'; connection: number; release: string; protocol: string }
  | {
      type: 'refuse'
      reason: 'version' | 'unauthorized'
      message: string
      sole: boolean
      clients: ClientSummary[]
    }
  | { type: 'result'; id: number; value?: unknown; error?: string }
  | { type: 'event'; name: string; payload: unknown }

/** A refusal from a core of another release. */
export class CoreRefused extends Error {
  readonly sole: boolean
  readonly clients: ClientSummary[]

  constructor(message: string, sole: boolean, clients: ClientSummary[]) {
    super(message)
    this.name = 'CoreRefused'
    this.sole = sole
    this.clients = clients
  }
}

export interface ServiceOptions {
  /** The profile directory. */
  dataDir: string
  /** The service binary shipped with this app. */
  binary: string
  /** The core's configuration (`ServiceConfig` in `crates/kchess-service`). */
  config: Record<string, unknown>
  /** Every core event, including `host:request`. */
  onEvent: (event: string, payload: unknown) => void
}

/** `{ release, protocol }` of the bundled binary: what this build sends in `hello`. */
function identity(binary: string): { release: string; protocol: string } {
  return JSON.parse(execFileSync(binary, ['version'], { encoding: 'utf8' })) as {
    release: string
    protocol: string
  }
}

/** The user's KChess runtime folder (`runtime_dir` in `crates/kchess-service`). */
function runtimeDir(): string {
  if (process.env.KCHESS_RUNTIME_DIR) return join(process.env.KCHESS_RUNTIME_DIR, 'core')
  if (process.platform === 'darwin')
    return join(homedir(), 'Library/Application Support/KChess Runtime/core')
  if (process.platform === 'win32')
    return join(process.env.LOCALAPPDATA ?? homedir(), 'KChess Runtime', 'core')
  return join(
    process.env.XDG_DATA_HOME ?? join(homedir(), '.local/share'),
    'kchess-runtime',
    'core',
  )
}

const digest = (path: string): string =>
  createHash('sha256').update(readFileSync(path)).digest('hex')

/** This release's copy of the binary in the runtime folder, made when missing or different. */
function install(binary: string, release: string): string {
  const dir = join(runtimeDir(), release)
  const target = join(dir, basename(binary))
  const wanted = digest(binary)
  if (existsSync(target) && digest(target) === wanted) return target
  mkdirSync(dir, { recursive: true })
  const temporary = join(dir, `.${basename(binary)}-${process.pid}`)
  copyFileSync(binary, temporary)
  if (digest(temporary) !== wanted) {
    rmSync(temporary, { force: true })
    throw new Error('The copied core service differs from the bundled one.')
  }
  renameSync(temporary, target)
  return target
}

/** What `core.json` says; none when no core runs for the profile. */
function readInfo(dataDir: string): CoreInfo | undefined {
  try {
    return JSON.parse(readFileSync(join(dataDir, 'core.json'), 'utf8')) as CoreInfo
  } catch (cause) {
    logDebug('core-service', 'No running core:', cause)
    return undefined
  }
}

const parts = (release: string): number[] => release.split('.').map((part) => Number(part) || 0)

function newer(a: string, b: string): boolean {
  const [x, y] = [parts(a), parts(b)]
  for (let i = 0; i < Math.max(x.length, y.length); i++)
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0)
  return false
}

/** What the person reads when another release has the profile open (`docs/shared-core.md`). */
export function refusalMessage(clients: ClientSummary[], ours: string, fallback: string): string {
  const other = clients[0]
  if (!other) return fallback
  const app = other.kind === 'gpui' ? 'GPUI' : other.kind === 'electron' ? 'Electron' : other.kind
  return newer(other.release, ours)
    ? `KChess ${other.release} (${app}) is open on this profile. Update this app to ${other.release} to use them together.`
    : `KChess ${other.release} (${app}) is open on this profile. Update it to ${ours}, or quit it, to open this app.`
}

/** A connection that said `hello` and was let in. */
interface Connection {
  socket: Socket
  connection: number
}

/** Connects to the running core, or reports why not: `undefined` when none runs. */
async function tryConnect(
  dataDir: string,
  own: { release: string; protocol: string },
  onFrame: (frame: Frame) => void,
  onClose: () => void,
): Promise<Connection | undefined> {
  const info = readInfo(dataDir)
  let token: string
  try {
    token = readFileSync(join(dataDir, 'core.token'), 'utf8').trim()
  } catch (cause) {
    logDebug('core-service', 'No core token:', cause)
    return undefined
  }
  if (!info) return undefined
  const socket = await new Promise<Socket | undefined>((resolve) => {
    const attempt = connect({ path: info.socket })
    attempt.once('connect', () => resolve(attempt))
    attempt.once('error', () => resolve(undefined))
  })
  if (!socket) return undefined
  return new Promise<Connection | undefined>((resolve, reject) => {
    let welcomed = false
    let buffered = Buffer.alloc(0)
    socket.on('data', (chunk: Buffer) => {
      buffered = Buffer.concat([buffered, chunk])
      while (buffered.length >= 4) {
        const length = buffered.readUInt32BE(0)
        if (length > MAX_FRAME) {
          socket.destroy()
          return
        }
        if (buffered.length < 4 + length) break
        const frame = JSON.parse(buffered.subarray(4, 4 + length).toString('utf8')) as Frame
        buffered = buffered.subarray(4 + length)
        if (welcomed) onFrame(frame)
        else if (frame.type === 'welcome') {
          welcomed = true
          resolve({ socket, connection: frame.connection })
        } else if (frame.type === 'refuse') {
          socket.destroy()
          if (frame.reason === 'version')
            reject(
              new CoreRefused(
                refusalMessage(frame.clients, own.release, frame.message),
                frame.sole,
                frame.clients,
              ),
            )
          else resolve(undefined)
        }
      }
    })
    socket.once('error', () => {
      if (!welcomed) resolve(undefined)
    })
    socket.once('close', () => {
      if (welcomed) onClose()
      else resolve(undefined)
    })
    send(socket, {
      type: 'hello',
      token,
      kind: 'electron',
      release: own.release,
      protocol: own.protocol,
      capabilities: CAPABILITIES,
    })
  })
}

function send(socket: Socket, frame: unknown): void {
  const body = Buffer.from(JSON.stringify(frame), 'utf8')
  const length = Buffer.alloc(4)
  length.writeUInt32BE(body.length, 0)
  socket.write(Buffer.concat([length, body]))
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

/**
 * The shared core as a `NativeCoreHandle`. It connects at once; calls wait for the connection.
 * `ready` settles when the first connection is made, or with the reason it could not be (a
 * `CoreRefused` names the app to update).
 */
export class ServiceCore implements NativeCoreHandle {
  readonly ready: Promise<void>
  private readonly own: { release: string; protocol: string }
  private current: Connection | undefined
  private connecting: Promise<Connection>
  private readonly pending = new Map<
    number,
    { resolve: (value: string) => void; reject: (cause: Error) => void }
  >()
  private next = 1
  private closed = false

  private readonly options: ServiceOptions

  constructor(options: ServiceOptions) {
    this.options = options
    this.own = identity(options.binary)
    this.connecting = this.connectOrStart()
    this.ready = this.connecting.then(() => undefined)
    // A failure is reported through `ready`; calls made meanwhile reject with it too.
    this.ready.catch((cause: unknown) => logDebug('core-service', 'Not connected:', cause))
  }

  private onFrame = (frame: Frame): void => {
    if (frame.type === 'result') {
      const waiting = this.pending.get(frame.id)
      if (!waiting) return
      this.pending.delete(frame.id)
      if (frame.error !== undefined) waiting.reject(new Error(frame.error))
      else waiting.resolve(JSON.stringify(frame.value ?? null))
    } else if (frame.type === 'event') this.options.onEvent(frame.name, frame.payload)
  }

  private onClose = (): void => {
    this.current = undefined
    for (const [, waiting] of this.pending) waiting.reject(new Error(STOPPED))
    this.pending.clear()
    if (this.closed) return
    logWarn('core-service', 'The core stopped; reconnecting.')
    this.connecting = this.reconnect()
  }

  private async reconnect(): Promise<Connection> {
    let delay = 200
    for (;;) {
      try {
        return await this.connectOrStart()
      } catch (cause) {
        logWarn('core-service', `The core could not restart: ${String(cause)}`)
        await sleep(delay)
        delay = Math.min(delay * 2, 5000)
      }
    }
  }

  /** `connect_or_start`: a running core first; else this release's copy, copied if needed. */
  private async connectOrStart(): Promise<Connection> {
    const attempt = () =>
      tryConnect(this.options.dataDir, this.own, this.onFrame, this.onClose).catch(
        (cause: unknown) => {
          if (cause instanceof CoreRefused && cause.sole) return 'replace' as const
          throw cause
        },
      )
    const first = await attempt()
    if (first && first !== 'replace') return (this.current = first)
    if (first === 'replace') {
      const deadline = Date.now() + START_DEADLINE_MS
      while (readInfo(this.options.dataDir) && Date.now() < deadline) await sleep(RETRY_MS)
    }
    this.launch()
    const deadline = Date.now() + START_DEADLINE_MS
    for (;;) {
      const connected = await attempt()
      if (connected && connected !== 'replace') return (this.current = connected)
      if (Date.now() > deadline) throw new Error('The KChess core did not start.')
      await sleep(RETRY_MS)
    }
  }

  /** Starts the service detached from this app, so closing the window does not stop it. */
  private launch(): void {
    let binary = this.options.binary
    try {
      binary = install(binary, this.own.release)
    } catch (cause) {
      logWarn('core-service', `Running the bundled core service: ${String(cause)}`)
    }
    const child = spawn(
      binary,
      ['serve', '--profile', this.options.dataDir, '--config', JSON.stringify(this.options.config)],
      { detached: true, stdio: 'ignore', windowsHide: true },
    )
    child.on('error', (cause) =>
      logError('core-service', `The core service could not start: ${cause.message}`),
    )
    child.unref()
  }

  async call(method: string, args: string): Promise<string> {
    const connection = this.current ?? (await this.connecting)
    const id = this.next++
    return new Promise<string>((resolve, reject) => {
      this.pending.set(id, { resolve, reject })
      send(connection.socket, { type: 'call', id, method, args: JSON.parse(args) as unknown[] })
    })
  }

  callSync(): string {
    throw new Error('Synchronous core calls are unavailable with the shared core.')
  }

  /** This client's connection id on the core; 0 until connected. */
  get connection(): number {
    return this.current?.connection ?? 0
  }

  /** Tells the core whether this app's window is focused. */
  focus(focused: boolean): void {
    if (this.current) send(this.current.socket, { type: 'focus', focused })
  }

  /** Leaves the shared core; it closes on its own once no window uses it. */
  async close(): Promise<void> {
    this.closed = true
    this.current?.socket.end()
    this.current = undefined
  }
}
