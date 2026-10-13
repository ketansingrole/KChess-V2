import { afterEach, expect, it, vi } from 'vitest'
import { flushPromises } from '@vue/test-utils'
import { QuitPreparation } from '../../electron/main/quit'
import { deferred } from './fixtures'
import { logWarn } from '@kchess/native/logger'

vi.mock('@kchess/native/logger', async (original) => ({
  ...(await original<typeof import('@kchess/native/logger')>()),
  logWarn: vi.fn(),
}))

afterEach(() => vi.useRealTimers())

function fixture() {
  const tokens: string[] = []
  const closed = new Set<() => void>()
  const retry = vi.fn(async () => false)
  const warn = vi.mocked(logWarn)
  const preparation = new QuitPreparation({
    window: () => ({
      send: (token) => tokens.push(token),
      onClosed(callback) {
        closed.add(callback)
        return () => {
          closed.delete(callback)
        }
      },
    }),
    retry,
    timeoutMs: 100,
  })
  return { preparation, tokens, closed, retry, warn }
}

it('only authorizes closing after a successful renderer flush', async () => {
  const f = fixture()
  const ready = f.preparation.prepare()
  expect(f.preparation.prepare()).toBe(ready)
  f.preparation.complete(f.tokens[0]!, true)
  expect(await ready).toBe(true)
  expect(f.retry).not.toHaveBeenCalled()
  expect(f.closed.size).toBe(0)
})

it('keeps quit pending on a failed save and cancels without closing', async () => {
  const f = fixture()
  const choice = deferred<boolean>()
  f.retry.mockImplementationOnce(() => choice.promise)
  let done = false
  const ready = f.preparation.prepare().then((saved) => {
    done = true
    return saved
  })
  f.preparation.complete(f.tokens[0]!, false)
  await flushPromises()
  expect(done).toBe(false)
  choice.resolve(false)
  expect(await ready).toBe(false)
  const next = f.preparation.prepare()
  f.preparation.complete(f.tokens[1]!, true)
  expect(await next).toBe(true)
})

it('retry requires a fresh successful flush and rejects old acknowledgments', async () => {
  const f = fixture()
  f.retry.mockResolvedValueOnce(true)
  const ready = f.preparation.prepare()
  f.preparation.complete(f.tokens[0]!, false)
  await flushPromises()
  expect(f.tokens).toHaveLength(2)
  expect(() => f.preparation.complete(f.tokens[0]!, true)).toThrow('No matching quit request')
  f.preparation.complete(f.tokens[1]!, true)
  expect(await ready).toBe(true)
})

it('a timeout cannot silently authorize quitting', async () => {
  vi.useFakeTimers()
  const f = fixture()
  const ready = f.preparation.prepare()
  await vi.advanceTimersByTimeAsync(100)
  expect(await ready).toBe(false)
  expect(f.warn).toHaveBeenCalledWith('shutdown', 'Renderer flush timed out.')
  expect(f.retry).toHaveBeenCalledOnce()
  expect(f.closed.size).toBe(0)
})

it('an error presenting the retry choice leaves quitting cancelled', async () => {
  const f = fixture()
  f.retry.mockRejectedValueOnce(new Error('Dialog unavailable'))
  const ready = f.preparation.prepare()
  f.preparation.complete(f.tokens[0]!, false)
  expect(await ready).toBe(false)
  expect(f.warn).toHaveBeenCalledWith('shutdown', 'Quit preparation failed.', expect.any(Error))
})

it('authorizes quitting immediately when the window is already gone', async () => {
  const retry = vi.fn(async () => false)
  const preparation = new QuitPreparation({ window: () => undefined, retry, timeoutMs: 100 })
  await expect(preparation.prepare()).resolves.toBe(true)
  expect(retry).not.toHaveBeenCalled()
})

it('cancels quitting when the window closes before the renderer answers', async () => {
  const f = fixture()
  const ready = f.preparation.prepare()
  await flushPromises()
  expect(f.tokens).toHaveLength(1)
  expect(f.closed.size).toBe(1)
  // Renderer crash or window closed mid-flush resolves the pending flush as failed.
  for (const onClose of [...f.closed]) onClose()
  expect(await ready).toBe(false)
  expect(f.retry).toHaveBeenCalledOnce()
  expect(f.closed.size).toBe(0)
})
