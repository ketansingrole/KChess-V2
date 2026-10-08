import { EventEmitter } from 'node:events'
import { PassThrough } from 'node:stream'
import type { ChildProcessByStdio } from 'node:child_process'
import type { Readable, Writable } from 'node:stream'
import { describe, expect, it, vi } from 'vitest'
import { UciController, ensureEngineOptions } from '../../src/core/uci'

function fake() {
  const written: string[] = []
  let syncs = 0
  const child = Object.assign(new EventEmitter(), {
    stdin: new PassThrough(),
    stdout: new PassThrough(),
    stderr: new PassThrough(),
    kill: vi.fn(() => {
      child.emit('exit', 0)
      return true
    }),
  })
  const engine = new UciController(
    child as unknown as ChildProcessByStdio<Writable, Readable, Readable>,
  )
  const origWrite = engine.write.bind(engine)
  engine.write = (command: string): void => {
    written.push(command)
    origWrite(command)
  }
  const origSync = engine.sync.bind(engine)
  engine.sync = async (): Promise<string> => {
    syncs++
    return origSync()
  }
  child.stdin.on('data', (chunk) => {
    const line = String(chunk).trim()
    if (line === 'uci') child.stdout.write('uciok\n')
    else if (line === 'isready') child.stdout.write('readyok\n')
  })
  return { engine, written, syncCount: (): number => syncs, child }
}

describe('engine option confirmation', () => {
  it('sends each option once, then skips redundant isready round-trips', async () => {
    const { engine, written, syncCount, child } = fake()
    await engine.ready
    written.length = 0
    const base = syncCount()
    const setoptions = (): string[] => written.filter((line) => line.startsWith('setoption'))

    // Review's per-position pattern: same Threads 140 times for one game.
    await ensureEngineOptions(engine, { Threads: '1' })
    expect(setoptions()).toEqual(['setoption name Threads value 1'])
    expect(syncCount()).toBe(base + 1)

    written.length = 0
    for (let i = 0; i < 20; i++) await ensureEngineOptions(engine, { Threads: '1' })
    expect(setoptions()).toEqual([])
    expect(syncCount()).toBe(base + 1)

    // Analysis scrubbing with identical options across positions.
    await ensureEngineOptions(engine, {
      Threads: '4',
      Hash: '128',
      MultiPV: '3',
      UCI_Chess960: 'true',
    })
    expect(syncCount()).toBe(base + 2)
    written.length = 0
    const before = syncCount()
    for (let i = 0; i < 10; i++)
      await ensureEngineOptions(engine, {
        Threads: '4',
        Hash: '128',
        MultiPV: '3',
        UCI_Chess960: 'true',
      })
    expect(setoptions()).toEqual([])
    expect(syncCount()).toBe(before)

    // Only the changed option is resent (battery saver 4 -> 2 threads).
    written.length = 0
    await ensureEngineOptions(engine, {
      Threads: '2',
      Hash: '128',
      MultiPV: '3',
      UCI_Chess960: 'true',
    })
    expect(setoptions()).toEqual(['setoption name Threads value 2'])
    expect(syncCount()).toBe(before + 1)
    engine.close()
    child.emit('exit', 0)
  })

  it('forgets confirmed options when the engine is closed', async () => {
    const { engine, written, syncCount, child } = fake()
    await engine.ready
    written.length = 0
    const base = syncCount()
    await ensureEngineOptions(engine, { Threads: '1' })
    expect(syncCount()).toBe(base + 1)
    engine.close()
    child.emit('exit', 0)
    // A replacement process starts empty: the same options are sent again.
    const next = fake()
    await next.engine.ready
    next.written.length = 0
    const nextBase = next.syncCount()
    await ensureEngineOptions(next.engine, { Threads: '1' })
    expect(next.written.filter((line) => line.startsWith('setoption'))).toEqual([
      'setoption name Threads value 1',
    ])
    expect(next.syncCount()).toBe(nextBase + 1)
    next.engine.close()
    next.child.emit('exit', 0)
    expect(written.length).toBeGreaterThan(0)
  })
})
