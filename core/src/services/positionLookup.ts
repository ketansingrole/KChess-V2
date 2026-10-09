import * as v from 'valibot'
import { Position } from '../domain/position'
import {
  EXPLORER_RATINGS,
  EXPLORER_SPEEDS,
  POSITION_LOOKUP_KINDS,
  type LookupOptions,
  type PositionLookup,
  type PositionLookupKind,
} from '../contracts/types'
import { assertAnalysisRequest, assertLookupOptions } from '../domain/validate'
import { logDebug } from './logger'

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
const explorerPlayer = v.looseObject({
  name: v.pipe(v.string(), v.maxLength(80)),
  rating: v.optional(v.nullable(v.number())),
})
const explorerGame = v.looseObject({
  id: v.pipe(v.string(), v.regex(/^[a-zA-Z0-9]{8}$/)),
  winner: v.optional(v.nullable(v.picklist(['white', 'black']))),
  white: explorerPlayer,
  black: explorerPlayer,
  year: v.optional(v.nullable(v.number())),
  month: v.optional(v.nullable(v.pipe(v.string(), v.maxLength(10)))),
  uci: v.optional(v.pipe(v.string(), v.maxLength(5))),
})
const openingSchema = v.looseObject({
  opening: v.optional(
    v.nullable(v.object({ name: v.pipe(v.string(), v.maxLength(200)), eco: v.string() })),
  ),
  white: v.optional(count),
  draws: v.optional(count),
  black: v.optional(count),
  moves: v.pipe(
    v.array(v.looseObject({ ...move, white: count, draws: count, black: count })),
    v.maxLength(256),
  ),
  topGames: v.optional(v.pipe(v.array(explorerGame), v.maxLength(30))),
  recentGames: v.optional(v.pipe(v.array(explorerGame), v.maxLength(30))),
})
const tablebaseSchema = v.object({
  category: categories,
  dtz,
  moves: v.pipe(v.array(v.object({ ...move, category: categories, dtz })), v.maxLength(256)),
})

type Cached = Omit<PositionLookup, 'cached' | 'stale' | 'message'>
const cachedGame = v.object({
  id: v.pipe(v.string(), v.regex(/^[a-zA-Z0-9]{8}$/)),
  white: v.pipe(v.string(), v.maxLength(80)),
  black: v.pipe(v.string(), v.maxLength(80)),
  whiteRating: v.optional(v.number()),
  blackRating: v.optional(v.number()),
  winner: v.optional(v.picklist(['white', 'black'])),
  year: v.optional(v.number()),
  month: v.optional(v.pipe(v.string(), v.maxLength(10))),
  san: v.optional(v.pipe(v.string(), v.maxLength(32))),
})
export function decodeLookupCache(raw: unknown): Cached | undefined {
  const result = v.safeParse(
    v.object({
      kind: v.picklist(POSITION_LOOKUP_KINDS),
      fen: v.pipe(v.string(), v.maxLength(100)),
      fetchedAt: count,
      total: v.optional(count),
      opening: v.optional(v.pipe(v.string(), v.maxLength(250))),
      category: v.optional(categories),
      dtz,
      games: v.optional(v.pipe(v.array(cachedGame), v.maxLength(30))),
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

/** How long a saved lookup is fresh: master games change rarely, a player's games often. */
const FRESH_MS: Record<PositionLookupKind, number> = {
  opening: 86_400_000,
  masters: 30 * 86_400_000,
  player: 3_600_000,
  tablebase: 30 * 86_400_000,
}
const ENDPOINT: Record<PositionLookupKind, string> = {
  opening: 'https://explorer.lichess.org/lichess',
  masters: 'https://explorer.lichess.org/masters',
  player: 'https://explorer.lichess.org/player',
  tablebase: 'https://tablebase.lichess.org/standard',
}
/** The player explorer streams improving results; it may send several snapshots. */
const MAX_BYTES: Record<PositionLookupKind, number> = {
  opening: 512_000,
  masters: 512_000,
  player: 4_000_000,
  tablebase: 512_000,
}

/** The filters a database uses, in a stable order (they are part of the cache key). */
function normalizeOptions(kind: PositionLookupKind, options: LookupOptions): LookupOptions {
  if (kind === 'tablebase') return {}
  if (kind === 'masters') return options.since ? { since: options.since.slice(0, 4) } : {}
  const speeds = options.speeds?.length
    ? EXPLORER_SPEEDS.filter((speed) => options.speeds!.includes(speed))
    : undefined
  if (kind === 'opening')
    return {
      speeds: speeds ?? ['blitz', 'rapid', 'classical'],
      ratings: options.ratings?.length
        ? EXPLORER_RATINGS.filter((rating) => options.ratings!.includes(rating))
        : undefined,
      since: options.since,
    }
  if (!options.player) throw new Error('Choose whose games to explore.')
  return {
    player: options.player,
    color: options.color ?? 'white',
    speeds,
    modes: options.modes?.length ? options.modes : undefined,
    since: options.since,
  }
}

/** Explicit lookups, deduplicated and bounded, with validated legal moves and stale offline recovery. */
export class PositionLookupService {
  private pending = new Map<string, Promise<PositionLookup>>()
  constructor(
    private cache: Cache,
    private fetcher: typeof fetch,
    private now = Date.now,
  ) {}
  async lookup(rawKind: unknown, rawFen: unknown, rawOptions?: unknown): Promise<PositionLookup> {
    const kind = v.parse(v.picklist(POSITION_LOOKUP_KINDS), rawKind)
    const fen = assertAnalysisRequest({ fen: rawFen, lines: 1 }).fen
    const options = normalizeOptions(kind, assertLookupOptions(rawOptions))
    const position = Position.fromFen(fen)
    if (!position) throw new Error('That position is not legal.')
    if (kind === 'tablebase' && (position.pieceCount > 7 || position.hasCastlingRights()))
      throw new Error('Tablebases cover positions with up to seven pieces and no castling rights.')
    const filters = JSON.stringify(options)
    // The original keys stay readable for lookups saved before filters existed.
    const key =
      kind === 'tablebase' || (kind === 'opening' && filters === DEFAULT_OPENING)
        ? `${kind}:${fen}`
        : `${kind}:${filters}:${fen}`
    const candidate = decodeLookupCache(this.cache.read(key))
    const saved =
      candidate?.fen === fen &&
      candidate.kind === kind &&
      candidate.moves.every((entry) => position.play(entry.uci)?.san === entry.san)
        ? candidate
        : undefined
    if (saved && this.now() - saved.fetchedAt < FRESH_MS[kind])
      return { ...saved, cached: true, stale: false }
    const known = this.pending.get(key)
    if (known) return known
    if (this.pending.size >= 16)
      throw new Error('Too many position lookups are pending. Try again shortly.')
    const request = this.load(kind, fen, options, position, key, saved)
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
    options: LookupOptions,
    position: Position,
    key: string,
    saved?: Cached,
  ): Promise<PositionLookup> {
    try {
      const url = new URL(ENDPOINT[kind])
      url.searchParams.set('fen', fen)
      if (kind !== 'tablebase') {
        url.searchParams.set('moves', '12')
        url.searchParams.set('recentGames', '0')
      }
      if (kind === 'opening') {
        url.searchParams.set('variant', 'standard')
        url.searchParams.set('speeds', (options.speeds ?? []).join(','))
        if (options.ratings?.length) url.searchParams.set('ratings', options.ratings.join(','))
        url.searchParams.set('topGames', '0')
        if (options.since) url.searchParams.set('since', options.since)
      } else if (kind === 'masters') {
        url.searchParams.set('topGames', '8')
        if (options.since) url.searchParams.set('since', options.since)
      } else if (kind === 'player') {
        url.searchParams.set('player', options.player!)
        url.searchParams.set('color', options.color ?? 'white')
        if (options.speeds?.length) url.searchParams.set('speeds', options.speeds.join(','))
        if (options.modes?.length) url.searchParams.set('modes', options.modes.join(','))
        if (options.since) url.searchParams.set('since', options.since)
        url.searchParams.set('recentGames', '8')
      }
      const response = await this.fetcher(url, {
        // The player database is built on demand and can take a while for a new player.
        signal: AbortSignal.timeout(kind === 'player' ? 60_000 : 15_000),
      })
      if (response.status === 401 || response.status === 403)
        throw new Error('Connect a Lichess account in Settings to use the opening explorer.')
      if (response.status === 404 && kind === 'player')
        throw new Error(`No Lichess player named "${options.player}".`)
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
          if (bytes > MAX_BYTES[kind]) throw new Error('Position lookup returned too much data.')
          text += decoder.decode(value, { stream: true })
        }
        text += decoder.decode()
      } finally {
        await reader.cancel().catch((error: unknown) => {
          logDebug('position-lookup', 'Stream cancel failed:', error)
        })
        reader.releaseLock()
      }
      // The player database answers with a stream of ever more complete results: use the last.
      const body =
        kind === 'player'
          ? (text
              .split('\n')
              .map((line) => line.trim())
              .filter(Boolean)
              .at(-1) ?? '{}')
          : text
      const raw: unknown = JSON.parse(body)
      const legalSan = (uci: string): string => {
        const played = position.play(uci)
        if (!played) throw new Error('Position lookup returned an illegal move.')
        return played.san
      }
      let result: Cached
      if (kind === 'tablebase') {
        const data = v.parse(tablebaseSchema, raw)
        result = {
          kind,
          fen,
          fetchedAt: this.now(),
          category: data.category,
          dtz: data.dtz,
          moves: data.moves.map((entry) => ({ ...entry, san: legalSan(entry.uci) })),
        }
      } else {
        const data = v.parse(openingSchema, raw)
        const listed = [...(data.topGames ?? []), ...(data.recentGames ?? [])].slice(0, 12)
        result = {
          kind,
          fen,
          fetchedAt: this.now(),
          total: (data.white ?? 0) + (data.draws ?? 0) + (data.black ?? 0) || undefined,
          ...(data.opening ? { opening: `${data.opening.eco} · ${data.opening.name}` } : {}),
          moves: data.moves.map((entry) => ({
            uci: entry.uci,
            white: entry.white,
            draws: entry.draws,
            black: entry.black,
            san: legalSan(entry.uci),
          })),
          games: listed.map((game) => {
            return {
              id: game.id,
              white: game.white.name,
              black: game.black.name,
              whiteRating: game.white.rating ?? undefined,
              blackRating: game.black.rating ?? undefined,
              winner: game.winner ?? undefined,
              year: game.year ?? undefined,
              month: game.month ?? undefined,
              san: game.uci ? position.play(game.uci)?.san : undefined,
            }
          }),
        }
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

  /** A master game's PGN; master games never change, so a saved copy is reused. */
  async mastersGame(rawId: unknown): Promise<string> {
    const id = v.parse(v.pipe(v.string(), v.regex(/^[a-zA-Z0-9]{8}$/)), rawId)
    const response = await this.fetcher(`https://explorer.lichess.org/masters/pgn/${id}`, {
      signal: AbortSignal.timeout(15_000),
    })
    if (response.status === 401 || response.status === 403)
      throw new Error('Connect a Lichess account in Settings to use the opening explorer.')
    if (!response.ok) throw new Error(`The master game could not be loaded (${response.status}).`)
    const text = await response.text()
    if (text.length > 200_000 || !text.includes('[')) throw new Error('That game is not a PGN.')
    return text
  }
}

const DEFAULT_OPENING = JSON.stringify({ speeds: ['blitz', 'rapid', 'classical'] })
