import * as v from 'valibot'
import { Chess } from 'chessops/chess'
import { parseFen } from 'chessops/fen'
import { parseUci } from 'chessops/util'
import { makeSan } from 'chessops/san'
import type { PositionLookup, PositionLookupKind } from '../shared/types'
import { assertAnalysisRequest } from '../shared/validate'

const count = v.pipe(v.number(), v.safeInteger(), v.minValue(0))
const categories = v.picklist([
  'win',
  'unknown',
  'syzygy-win',
  'maybe-win',
  'cursed-win',
  'draw',
  'blessed-loss',
  'maybe-loss',
  'syzygy-loss',
  'loss',
])
const dtz = v.optional(v.nullable(v.pipe(v.number(), v.safeInteger())))
const move = { uci: v.pipe(v.string(), v.maxLength(5)) }
const openingSchema = v.object({
  opening: v.optional(
    v.nullable(v.object({ name: v.pipe(v.string(), v.maxLength(200)), eco: v.string() })),
  ),
  moves: v.pipe(
    v.array(v.object({ ...move, white: count, draws: count, black: count })),
    v.maxLength(256),
  ),
})
const tablebaseSchema = v.object({
  category: categories,
  dtz,
  moves: v.pipe(v.array(v.object({ ...move, category: categories, dtz })), v.maxLength(256)),
})
type Cached = Omit<PositionLookup, 'cached' | 'stale' | 'message'>
export function decodeLookupCache(raw: unknown): Cached | undefined {
  const result = v.safeParse(
    v.object({
      kind: v.picklist(['opening', 'tablebase']),
      fen: v.pipe(v.string(), v.maxLength(100)),
      fetchedAt: count,
      opening: v.optional(v.pipe(v.string(), v.maxLength(250))),
      category: v.optional(categories),
      dtz,
      moves: v.pipe(
        v.array(
          v.object({
            ...move,
            san: v.pipe(v.string(), v.maxLength(32)),
            white: v.optional(count),
            draws: v.optional(count),
            black: v.optional(count),
            category: v.optional(categories),
            dtz,
          }),
        ),
        v.maxLength(256),
      ),
    }),
    raw,
  )
  return result.success ? result.output : undefined
}
interface Cache {
  read(key: string): Cached | undefined
  write(key: string, value: Cached): void
}
/** Explicit lookups, deduplicated and bounded, with validated legal moves and stale offline recovery. */
export class PositionLookupService {
  private pending = new Map<string, Promise<PositionLookup>>()
  constructor(
    private cache: Cache,
    private fetcher: typeof fetch,
    private now = Date.now,
  ) {}
  async lookup(rawKind: unknown, rawFen: unknown): Promise<PositionLookup> {
    const kind = v.parse(v.picklist(['opening', 'tablebase']), rawKind)
    const fen = assertAnalysisRequest({ fen: rawFen, lines: 1 }).fen
    const position = Chess.fromSetup(parseFen(fen).unwrap()).unwrap()
    if (
      kind === 'tablebase' &&
      (position.board.occupied.size() > 7 || position.castles.castlingRights.nonEmpty())
    )
      throw new Error('Tablebases cover positions with up to seven pieces and no castling rights.')
    const key = `${kind}:${fen}`
    const candidate = decodeLookupCache(this.cache.read(key))
    const saved =
      candidate?.fen === fen &&
      candidate.kind === kind &&
      candidate.moves.every((entry) => {
        const move = parseUci(entry.uci)
        return move && position.isLegal(move) && entry.san === makeSan(position, move)
      })
        ? candidate
        : undefined
    if (saved && this.now() - saved.fetchedAt < (kind === 'opening' ? 86_400_000 : 30 * 86_400_000))
      return { ...saved, cached: true, stale: false }
    const known = this.pending.get(key)
    if (known) return known
    if (this.pending.size >= 16)
      throw new Error('Too many position lookups are pending. Try again shortly.')
    const request = this.load(kind, fen, position, key, saved)
    this.pending.set(key, request)
    try {
      return await request
    } finally {
      this.pending.delete(key)
    }
  }
  private async load(
    kind: PositionLookupKind,
    fen: string,
    position: Chess,
    key: string,
    saved?: Cached,
  ): Promise<PositionLookup> {
    try {
      const url = new URL(
        kind === 'opening'
          ? 'https://explorer.lichess.org/lichess'
          : 'https://tablebase.lichess.org/standard',
      )
      url.searchParams.set('fen', fen)
      if (kind === 'opening') {
        url.searchParams.set('variant', 'standard')
        url.searchParams.set('speeds', 'blitz,rapid,classical')
        url.searchParams.set('moves', '12')
        url.searchParams.set('topGames', '0')
        url.searchParams.set('recentGames', '0')
      }
      const response = await this.fetcher(url, { signal: AbortSignal.timeout(15_000) })
      if (response.status === 401 || response.status === 403)
        throw new Error('Connect a Lichess account in Settings to use the opening explorer.')
      if (!response.ok)
        throw new Error(`Position lookup returned ${response.status}. Try again shortly.`)
      if (!response.body) throw new Error('Position lookup returned no data.')
      const reader = response.body.getReader()
      let text = '',
        bytes = 0
      const decoder = new TextDecoder()
      try {
        for (;;) {
          const { done, value } = await reader.read()
          if (done) break
          bytes += value.byteLength
          if (bytes > 512_000) throw new Error('Position lookup returned too much data.')
          text += decoder.decode(value, { stream: true })
        }
        text += decoder.decode()
      } finally {
        await reader.cancel().catch(() => {})
        reader.releaseLock()
      }
      const raw: unknown = JSON.parse(text)
      const data = kind === 'opening' ? v.parse(openingSchema, raw) : v.parse(tablebaseSchema, raw)
      const result: Cached = {
        kind,
        fen,
        fetchedAt: this.now(),
        ...('opening' in data && data.opening
          ? { opening: `${data.opening.eco} · ${data.opening.name}` }
          : {}),
        ...('category' in data ? { category: data.category, dtz: data.dtz } : {}),
        moves: data.moves.map((entry) => {
          const move = parseUci(entry.uci)
          if (!move || !position.isLegal(move))
            throw new Error('Position lookup returned an illegal move.')
          return { ...entry, san: makeSan(position, move) }
        }),
      }
      this.cache.write(key, result)
      return { ...result, cached: false, stale: false }
    } catch (cause) {
      if (saved)
        return {
          ...saved,
          cached: true,
          stale: true,
          message: 'Could not refresh. Showing the saved lookup.',
        }
      throw cause instanceof Error
        ? cause
        : new Error('Position lookup failed. Check your connection and retry.')
    }
  }
}
