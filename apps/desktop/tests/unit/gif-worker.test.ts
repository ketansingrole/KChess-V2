import { expect, it, vi } from 'vitest'
import { GifWorkerClient } from '../../app/utils/gifWorkerClient'

function worker() {
  const target = {
    onmessage: null as ((event: MessageEvent) => void) | null,
    onerror: null as (() => void) | null,
    terminate: vi.fn(),
    postMessage: vi.fn(),
  }
  target.postMessage.mockImplementation((message: unknown, transfer: Transferable[]) =>
    structuredClone(message, { transfer }),
  )
  return { target, client: new GifWorkerClient(target as unknown as Worker, 8, 8) }
}
it('transfers pixels and waits for acknowledgment before producing the next frame', async () => {
  const { target, client } = worker()
  const rgba = new Uint8ClampedArray(8 * 8 * 4)
  const frame = client.add({ rgba, delay: 40 })
  expect(rgba.byteLength).toBe(0)
  await expect(client.add({ rgba: new Uint8ClampedArray(8 * 8 * 4), delay: 40 })).rejects.toThrow(
    'already being encoded',
  )
  target.onmessage?.({ data: { ready: true } } as MessageEvent)
  await frame
  const finish = client.finish(),
    result = new Uint8Array([71, 73, 70])
  target.onmessage?.({ data: { data: result } } as MessageEvent)
  expect(await finish).toBe(result)
  client.close()
  expect(target.terminate).toHaveBeenCalledTimes(1)
})
it('cancels active encoding immediately and releases the worker', async () => {
  const target = { onmessage: null, onerror: null, terminate: vi.fn(), postMessage: vi.fn() }
  const abort = new AbortController(),
    client = new GifWorkerClient(target as unknown as Worker, 8, 8, abort.signal)
  const frame = client.add({ rgba: new Uint8ClampedArray(8 * 8 * 4), delay: 40 })
  const rejection = expect(frame).rejects.toMatchObject({ name: 'AbortError' })
  abort.abort()
  await rejection
  expect(target.terminate).toHaveBeenCalledTimes(1)
  await expect(client.finish()).rejects.toMatchObject({ name: 'AbortError' })
  client.close()
})
it('propagates worker errors instead of leaving an export pending', async () => {
  const { target, client } = worker(),
    frame = client.add({ rgba: new Uint8ClampedArray(256), delay: 40 })
  const rejection = expect(frame).rejects.toThrow('encoder stopped')
  target.onerror?.()
  await rejection
  client.close()
})

it('bounds transferred frame ownership throughout a long export and terminates only once', async () => {
  const { target, client } = worker()
  for (let i = 0; i < 600; i++) {
    const rgba = new Uint8ClampedArray(256)
    const work = client.add({ rgba, delay: 40 })
    expect(rgba.byteLength).toBe(0)
    // No second frame can be retained while this acknowledgment is outstanding.
    await expect(client.add({ rgba: new Uint8ClampedArray(256), delay: 40 })).rejects.toThrow(
      'already being encoded',
    )
    target.onmessage?.({ data: { ready: true } } as MessageEvent)
    await work
    target.postMessage.mockClear()
  }
  client.close()
  client.close()
  expect(target.terminate).toHaveBeenCalledTimes(1)
  expect(target.onmessage).toBeNull()
  expect(target.onerror).toBeNull()
})
