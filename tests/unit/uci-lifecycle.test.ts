import { EventEmitter } from 'node:events'
import { PassThrough } from 'node:stream'
import type { ChildProcessByStdio } from 'node:child_process'
import type { Readable, Writable } from 'node:stream'
import { describe, expect, it, vi } from 'vitest'
import { UciController, SearchCancelled } from '../../src/main/uci'
import { acquireEngine } from '../../src/main/engineScheduler'
function fake(reply: (command: string) => string | undefined) {
  const child = Object.assign(new EventEmitter(), {
    stdin: new PassThrough(),
    stdout: new PassThrough(),
    stderr: new PassThrough(),
    kill: vi.fn(() => {
      child.emit('exit', 0)
      return true
    }),
  })
  child.stdin.on('data', (chunk) => {
    const line = reply(String(chunk).trim())
    if (line) child.stdout.write(`${line}\n`)
  })
  return child as unknown as ChildProcessByStdio<Writable, Readable, Readable>
}
const handshake = (command: string): string | undefined =>
  command === 'uci' ? 'uciok' : command === 'isready' ? 'readyok' : undefined

describe('UCI failure recovery', () => {
  it('bounds a silent startup and terminates its process', async () => {
    const child = fake(() => undefined)
    const engine = new UciController(child, 20)
    await expect(engine.ready).rejects.toThrow('timed out')
    expect(child.kill).toHaveBeenCalled()
  })
  it('rejects an ignored readiness request', async () => {
    const engine = new UciController(
      fake((command) => (command === 'uci' ? 'uciok' : undefined)),
      20,
    )
    await expect(engine.ready).rejects.toThrow('timed out')
  })
  it('cancels an active search after the stop acknowledgment', async () => {
    const engine = new UciController(
      fake((command) => (command === 'stop' ? 'bestmove e2e4' : handshake(command))),
      20,
    )
    await engine.ready
    const controller = new AbortController()
    const search = engine.search('go infinite', () => {}, 0, controller.signal)
    const rejected = expect(search).rejects.toBeInstanceOf(SearchCancelled)
    controller.abort()
    await rejected
    engine.close()
  })
  it('kills a search that ignores stop instead of waiting indefinitely', async () => {
    const child = fake(handshake)
    const engine = new UciController(child, 20)
    await engine.ready
    const controller = new AbortController()
    const search = engine.search('go infinite', () => {}, 0, controller.signal)
    const rejected = expect(search).rejects.toThrow('did not stop')
    controller.abort()
    await rejected
    expect(child.kill).toHaveBeenCalled()
  }, 4_000)
  it('rejects a bounded search with no bestmove', async () => {
    const engine = new UciController(fake(handshake), 20)
    await engine.ready
    await expect(engine.search('go movetime 1', () => {}, 20)).rejects.toThrow('timed out')
  })
})
describe('shared engine budget', () => {
  it('preempts lower-priority work and releases queued work in priority order', async () => {
    const stop = vi.fn()
    const release = await acquireEngine(1, stop)
    const analysis = acquireEngine(2, () => {})
    const computer = acquireEngine(3, () => {})
    expect(stop).toHaveBeenCalled()
    release()
    const releaseComputer = await computer
    let started = false
    void analysis.then(() => {
      started = true
    })
    await Promise.resolve()
    expect(started).toBe(false)
    releaseComputer()
    const releaseAnalysis = await analysis
    releaseAnalysis()
  })
  it('removes cancelled queued work', async () => {
    const release = await acquireEngine(3, () => {})
    const controller = new AbortController()
    const queued = acquireEngine(1, () => {}, controller.signal)
    const rejected = expect(queued).rejects.toBeInstanceOf(SearchCancelled)
    controller.abort()
    await rejected
    release()
  })
})
