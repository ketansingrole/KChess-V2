import { describe, it, expect, vi } from 'vitest'
import { INITIAL_FEN } from 'chessops/fen'
import { PositionLookupService } from '../../src/services/positionLookup'
import type { PositionLookup } from '../../src/contracts/types'
import { deferred } from '../../../tests/fixtures/deferred'

function fixture(fetcher: typeof fetch, now = () => 1_000) {
  const values = new Map<string, PositionLookup>()
  const service = new PositionLookupService(
    {
      read: (key) => values.get(key),
      write: (key, value) => values.set(key, { ...value, cached: false, stale: false }),
    },
    fetcher,
    now,
  )
  return { service, values }
}
const opening = { moves: [{ uci: 'e2e4', white: 100, draws: 20, black: 50 }] }
describe('position lookup service', () => {
  it('deduplicates lookups, validates legal SAN and reuses the saved result', async () => {
    const pending = deferred<Response>()
    const fetcher = vi.fn<typeof fetch>().mockReturnValue(pending.promise)
    const { service } = fixture(fetcher)
    const one = service.lookup('opening', INITIAL_FEN)
    const two = service.lookup('opening', INITIAL_FEN)
    expect(fetcher).toHaveBeenCalledTimes(1)
    pending.resolve(Response.json(opening))
    expect((await one).moves[0]?.san).toBe('e4')
    expect(await two).toEqual(await one)
    expect((await service.lookup('opening', INITIAL_FEN)).cached).toBe(true)
    expect(fetcher).toHaveBeenCalledTimes(1)
  })
  it('shows a saved result as stale after a failed refresh', async () => {
    let now = 1_000
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json(opening))
      .mockRejectedValueOnce(new Error('offline'))
    const { service } = fixture(fetcher, () => now)
    await service.lookup('opening', INITIAL_FEN)
    now += 2 * 86_400_000
    const result = await service.lookup('opening', INITIAL_FEN)
    expect(result).toMatchObject({ stale: true, cached: true })
    expect(result.message).toContain('saved')
  })
  it.each([
    { moves: [{ uci: 'e2e5', white: 10, draws: 0, black: 1 }] },
    { moves: [{ uci: 'e2e4', white: -10, draws: 0, black: 1 }] },
    { moves: Array(257).fill(opening.moves[0]) },
  ])('rejects malformed and illegal remote moves', async (data) => {
    const { service } = fixture(vi.fn<typeof fetch>().mockResolvedValue(Response.json(data)))
    await expect(service.lookup('opening', INITIAL_FEN)).rejects.toThrow()
  })
  it('bounds response size and rejects unsupported tablebase positions before fetching', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response('x'.repeat(512_001)))
    const { service } = fixture(fetcher)
    await expect(service.lookup('tablebase', INITIAL_FEN)).rejects.toThrow('seven pieces')
    expect(fetcher).not.toHaveBeenCalled()
    await expect(service.lookup('opening', INITIAL_FEN)).rejects.toThrow('too much data')
  })
  it('retains tablebase fifty-move categories and DTZ without inventing a mate distance', async () => {
    const { service } = fixture(
      vi
        .fn<typeof fetch>()
        .mockResolvedValue(Response.json({ category: 'cursed-win', dtz: 101, moves: [] })),
    )
    expect(await service.lookup('tablebase', '8/8/8/8/8/8/R7/K6k w - - 0 1')).toMatchObject({
      category: 'cursed-win',
      dtz: 101,
    })
  })
})
