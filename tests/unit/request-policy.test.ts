import { describe, expect, it, vi } from 'vitest'
import { lichessFetch } from '../../src/main/requestPolicy'

const state = vi.hoisted(() => ({ fetch: vi.fn() }))
vi.mock('../../src/main/usage', () => ({
  meteredFetch: (request: Request) => state.fetch(request),
}))

/** An ndjson body that stays open until `end()` is called. */
function openStream(): { response: Response; end: () => void } {
  let close = (): void => {}
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode('{}\n'))
      close = () => controller.close()
    },
  })
  return {
    response: new Response(body, { headers: { 'Content-Type': 'application/x-ndjson' } }),
    end: () => close(),
  }
}

const settled = async (promise: Promise<unknown>): Promise<boolean> => {
  let done = false
  void promise.then(
    () => (done = true),
    () => (done = true),
  )
  for (let i = 0; i < 10; i++) await new Promise((resolve) => setTimeout(resolve, 0))
  return done
}

describe('Lichess request policy', () => {
  it('runs one game export at a time until its body ends, without blocking other requests', async () => {
    const first = openStream()
    state.fetch.mockImplementation(async (request: Request) =>
      request.url.includes('/api/games/user/Alice')
        ? first.response
        : new Response('{}', { headers: { 'Content-Type': 'application/json' } }),
    )
    const response = await lichessFetch('https://lichess.org/api/games/user/Alice')
    const reader = response.body!.getReader()
    await reader.read()

    const second = lichessFetch('https://lichess.org/api/games/export/_ids', { method: 'POST' })
    expect(await settled(second)).toBe(false)
    // Interactive traffic is not held behind the open export.
    expect((await lichessFetch('https://lichess.org/api/account')).ok).toBe(true)

    first.end()
    expect((await reader.read()).done).toBe(true)
    expect(await settled(second)).toBe(true)
    await (await second).text()
  })
  it('releases the export lane when a body is cancelled or the request fails', async () => {
    const stream = openStream()
    state.fetch.mockResolvedValueOnce(stream.response)
    const response = await lichessFetch('https://lichess.org/api/games/user/Alice')
    await response.body!.cancel()

    state.fetch.mockRejectedValueOnce(new Error('offline'))
    await expect(lichessFetch('https://lichess.org/api/games/user/Alice')).rejects.toThrow(
      'offline',
    )

    state.fetch.mockResolvedValueOnce(new Response('', { status: 500 }))
    const failed = await lichessFetch('https://lichess.org/api/games/user/Alice')
    expect(failed.status).toBe(500)

    state.fetch.mockResolvedValueOnce(new Response('{}\n'))
    expect(await settled(lichessFetch('https://lichess.org/api/games/user/Alice'))).toBe(true)
  })
})
