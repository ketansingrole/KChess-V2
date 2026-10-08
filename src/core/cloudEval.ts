import * as v from 'valibot'
import { LRUCache } from 'lru-cache'
import type { CloudEval } from '../shared/types'
import { replay } from '../shared/review'
import { LichessError } from '../shared/lichessError'
import { client, unwrap } from './lichess'
import { withUsage } from './usage'

const pvSchema = v.union([
  v.looseObject({
    cp: v.pipe(v.number(), v.finite()),
    moves: v.pipe(v.string(), v.maxLength(2000)),
  }),
  v.looseObject({
    mate: v.pipe(v.number(), v.finite()),
    moves: v.pipe(v.string(), v.maxLength(2000)),
  }),
])
const schema = v.looseObject({
  depth: v.pipe(v.number(), v.finite(), v.minValue(0)),
  knodes: v.pipe(v.number(), v.finite(), v.minValue(0)),
  pvs: v.pipe(v.array(pvSchema), v.maxLength(5)),
})

/** Positions Lichess has no evaluation for are remembered too, so they are not asked again soon. */
const cache = new LRUCache<string, CloudEval | 'none'>({ max: 500, ttl: 10 * 60_000 })

/**
 * Lichess's cloud evaluation, which exists only for positions someone analysed deeply on
 * Lichess before. Lines are checked to be legal from the position; unknown positions give null.
 */
export async function cloudEval(fen: string, lines: number): Promise<CloudEval | null> {
  const key = `${lines}:${fen}`
  const hit = cache.get(key)
  if (hit) return hit === 'none' ? null : hit
  try {
    const raw = await withUsage('', 'analysis', () =>
      unwrap(client.GET('/api/cloud-eval', { params: { query: { fen, multiPv: lines } } })),
    )
    const data = v.parse(schema, raw)
    const result: CloudEval = {
      fen,
      depth: data.depth,
      knodes: data.knodes,
      lines: data.pvs.flatMap((pv, index) => {
        const moves = pv.moves.trim().split(/\s+/).slice(0, 30)
        // Keep the legal prefix; an illegal first move drops the line.
        const legal = replay(fen, moves).length - 1
        if (legal < 1) return []
        return [
          {
            rank: index + 1,
            depth: data.depth,
            ...(typeof pv.cp === 'number' ? { cp: pv.cp } : { mate: pv.mate as number }),
            pv: moves.slice(0, legal),
          },
        ]
      }),
    }
    cache.set(key, result)
    return result
  } catch (cause) {
    if (cause instanceof LichessError && cause.status === 404) {
      cache.set(key, 'none')
      return null
    }
    throw cause
  }
}
