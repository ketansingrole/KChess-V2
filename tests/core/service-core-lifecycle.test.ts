import { EventEmitter } from 'node:events'
import { afterEach, expect, it, vi } from 'vitest'
import { ServiceCore } from '../../crates/kchess-node/js/serviceCore'

/** The shared core's client over a socket the test controls (`crates/kchess-node/js/serviceCore.ts`). */
const fixture = vi.hoisted(() => ({ socket: undefined as unknown }))
vi.mock('node:child_process', async (original) => {
  const actual = await original<typeof import('node:child_process')>()
  const execFileSync = () => JSON.stringify({ release: '2026.10.12', protocol: 'test' })
  const spawn = () => Object.assign(new EventEmitter(), { unref: () => {} })
  return { ...actual, execFileSync, spawn, default: { ...actual, execFileSync, spawn } }
})
vi.mock('node:fs', async (original) => {
  const fs = await original<typeof import('node:fs')>()
  const readFileSync = (...args: Parameters<typeof fs.readFileSync>) => {
    const path = String(args[0])
    if (path === '/profile/core.json') return JSON.stringify({ socket: '/profile/socket' })
    if (path === '/profile/core.token') return 'token'
    return fs.readFileSync(...args)
  }
  return { ...fs, readFileSync, default: { ...fs, readFileSync } }
})
vi.mock('node:net', async (original) => {
  const actual = await original<typeof import('node:net')>()
  const connect = () => {
    const socket = fixture.socket as EventEmitter
    queueMicrotask(() => socket.emit('connect'))
    return socket
  }
  return { ...actual, connect, default: { ...actual, connect } }
})

function peer() {
  const socket = Object.assign(new EventEmitter(), {
    write: vi.fn(),
    end: vi.fn(),
    destroy: vi.fn(function (this: EventEmitter, cause?: Error) {
      if (cause) this.emit('error', cause)
      this.emit('close')
    }),
  })
  fixture.socket = socket
  return socket
}

function welcome(socket: EventEmitter, connection: number): void {
  const body = Buffer.from(
    JSON.stringify({ type: 'welcome', connection, release: '2026.10.12', protocol: 'test' }),
  )
  const header = Buffer.alloc(4)
  header.writeUInt32BE(body.length)
  socket.emit('data', Buffer.concat([header, body]))
}

const hello = async (socket: { write: ReturnType<typeof vi.fn> }) => {
  for (let n = 0; n < 20 && !socket.write.mock.calls.length; n++) await Promise.resolve()
  expect(socket.write).toHaveBeenCalled()
}

const service = () =>
  new ServiceCore({ dataDir: '/profile', binary: '/binary', config: {}, onEvent: vi.fn() })

afterEach(() => void vi.useRealTimers())

it('a client closed during the handshake never takes the connection it was making', async () => {
  const socket = peer()
  const core = service()
  await hello(socket)
  await core.close()
  welcome(socket, 7)
  await expect(core.ready).rejects.toThrow('The KChess core stopped.')
  expect(core.connection).toBe(0)
  await expect(core.call('library', '[]')).rejects.toThrow('The KChess core stopped.')
})

it('a closed client ends its connection and refuses later calls', async () => {
  const socket = peer()
  const core = service()
  await hello(socket)
  welcome(socket, 3)
  await core.ready
  const waiting = core.call('library', '[]')
  await core.close()
  expect(socket.end).toHaveBeenCalled()
  await expect(waiting).rejects.toThrow('The KChess core stopped.')
  await expect(core.call('library', '[]')).rejects.toThrow('The KChess core stopped.')
})

it('a core that never answers hello fails the connection instead of waiting', async () => {
  vi.useFakeTimers()
  const socket = peer()
  const core = service()
  await hello(socket)
  const ready = expect(core.ready).rejects.toThrow('The KChess core did not answer.')
  await vi.advanceTimersByTimeAsync(5000)
  await ready
  expect(socket.destroy).toHaveBeenCalled()
  await core.close()
})

it('after the core restarts, the window’s focus is sent again and its owners hear of it', async () => {
  const first = peer()
  const onEvent = vi.fn()
  const core = new ServiceCore({ dataDir: '/profile', binary: '/binary', config: {}, onEvent })
  await hello(first)
  welcome(first, 1)
  await core.ready
  core.focus(true)
  const second = peer()
  first.emit('close')
  await hello(second)
  welcome(second, 2)
  await vi.waitFor(() => expect(onEvent).toHaveBeenCalledWith('core:reconnected', null))
  expect(core.connection).toBe(2)
  const frames = second.write.mock.calls.map(([chunk]) =>
    JSON.parse((chunk as Buffer).subarray(4).toString('utf8')),
  )
  expect(frames).toContainEqual({ type: 'focus', focused: true })
  await core.close()
})
