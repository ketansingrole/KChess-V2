import * as v from 'valibot'
import { makeFen } from 'chessops/fen'
import { makeUci } from 'chessops/util'
import { parseComment, parsePgn, startingPosition } from 'chessops/pgn'
import { parseSan } from 'chessops/san'
import type {
  BroadcastGame,
  BroadcastSummary,
  BroadcastTourDetail,
  BroadcastUpdate,
  TvChannel,
  TvPastGame,
  WatchFrame,
  WatchState,
  WatchPlayer,
} from '../shared/types'
import {
  defaultFen,
  replaySetup,
  setupStart,
  variantFromLichess,
  type GameSetup,
} from '../shared/variant'
import { client, unwrap } from './lichess'
import { readLines } from './ndjson'
import { withUsage } from './usage'

/** Lichess TV channels, keyed as the feed URLs name them. */
export const TV_CHANNELS: readonly { key: string; label: string }[] = [
  { key: 'best', label: 'Top rated' },
  { key: 'bullet', label: 'Bullet' },
  { key: 'blitz', label: 'Blitz' },
  { key: 'rapid', label: 'Rapid' },
  { key: 'classical', label: 'Classical' },
  { key: 'ultraBullet', label: 'UltraBullet' },
  { key: 'chess960', label: 'Chess960' },
  { key: 'kingOfTheHill', label: 'King of the Hill' },
  { key: 'threeCheck', label: 'Three-check' },
  { key: 'antichess', label: 'Antichess' },
  { key: 'atomic', label: 'Atomic' },
  { key: 'horde', label: 'Horde' },
  { key: 'racingKings', label: 'Racing Kings' },
  { key: 'crazyhouse', label: 'Crazyhouse' },
  { key: 'bot', label: 'Bots' },
  { key: 'computer', label: 'Computer' },
]
export const TV_CHANNEL_KEYS = TV_CHANNELS.map((channel) => channel.key)

const text = (max: number) => v.pipe(v.string(), v.maxLength(max))
const num = v.pipe(v.number(), v.finite())
const lightUser = v.looseObject({
  name: v.optional(text(40)),
  id: v.optional(text(40)),
  title: v.optional(v.nullable(text(8))),
})

/** Board and side to move only: Crazyhouse pockets and odd fields never reach the board. */
export function displayFen(raw: string): string | undefined {
  const [board = '', turn = 'w', castling = '-', ep = '-'] = raw.trim().split(/\s+/)
  const ranks = board
    .replace(/\[[^\]]*\]$/, '')
    .split('/')
    .slice(0, 8)
  if (ranks.length !== 8 || !ranks.every((rank) => /^[1-8pnbrqkPNBRQK~]{1,16}$/.test(rank)))
    return undefined
  return `${ranks.join('/').replace(/~/g, '')} ${turn === 'b' ? 'b' : 'w'} ${/^[KQkqA-Ha-h]{1,4}$|^-$/.test(castling) ? castling : '-'} ${/^[a-h][36]$|^-$/.test(ep) ? ep : '-'} 0 1`
}
const uciOrUndefined = (value: unknown): string | undefined =>
  typeof value === 'string' && /^[a-h][1-8][a-h][1-8][qrbnk]?$|^[PNBRQ]@[a-h][1-8]$/.test(value)
    ? value
    : undefined

const playerOf = (raw: unknown): WatchPlayer => {
  const parsed = v.safeParse(
    v.looseObject({
      user: v.optional(lightUser),
      rating: v.optional(num),
      aiLevel: v.optional(num),
      name: v.optional(text(40)),
    }),
    raw,
  )
  if (!parsed.success) return { name: '?' }
  const player = parsed.output
  return {
    name:
      player.user?.name ??
      player.name ??
      (player.aiLevel ? `Stockfish level ${player.aiLevel}` : 'Anonymous'),
    title: player.user?.title ?? undefined,
    rating: player.rating,
  }
}

const gameDetails = v.looseObject({
  rated: v.optional(v.boolean()),
  speed: v.optional(text(20)),
  clock: v.optional(v.looseObject({ initial: num, increment: num })),
})
type GameDetails = Pick<WatchFrame, 'rated' | 'speed' | 'clock'>
/** Rated flag, speed and time control of a game: fields the TV feed leaves out. */
export function parseGameDetails(raw: unknown): GameDetails {
  const parsed = v.safeParse(gameDetails, raw)
  if (!parsed.success) return {}
  const { rated, speed, clock } = parsed.output
  return {
    rated,
    speed,
    clock: clock ? { initial: clock.initial, increment: clock.increment } : undefined,
  }
}

const exportedGame = v.looseObject({
  moves: v.optional(text(20_000)),
  initialFen: v.optional(text(120)),
  variant: v.optional(text(20)),
  status: v.optional(text(30)),
  winner: v.optional(v.picklist(['white', 'black'])),
})
interface TvGame {
  details: GameDetails
  /** Undefined for variants KChess cannot replay (Crazyhouse). */
  setup?: GameSetup
  san: string[]
  status?: string
  winner?: 'white' | 'black'
}
/** A game's details, start and SAN moves from its JSON export (ongoing games lag a few moves). */
export function parseTvGame(raw: unknown): TvGame {
  const parsed = v.safeParse(exportedGame, raw)
  if (!parsed.success) return { details: parseGameDetails(raw), san: [] }
  const game = parsed.output
  const variant = variantFromLichess(game.variant ?? 'standard')
  return {
    details: parseGameDetails(raw),
    setup: variant ? { variant, fen: game.initialFen ?? defaultFen(variant) } : undefined,
    san: game.moves ? game.moves.split(' ').filter(Boolean) : [],
    status: game.status,
    winner: game.winner,
  }
}
async function fetchTvGame(id: string, signal: AbortSignal): Promise<TvGame> {
  const raw = await unwrap(
    client.GET('/game/export/{gameId}', {
      params: {
        path: { gameId: id },
        query: { moves: true, tags: false, clocks: false, evals: false, opening: false },
      },
      headers: { Accept: 'application/json' },
      signal,
    }),
  )
  return parseTvGame(raw)
}

/** Board and side to move: what both the feed's positions and a replay can be compared on. */
const positionKey = (fen: string): string => fen.split(' ').slice(0, 2).join(' ')

/**
 * Lichess exports an ongoing game a few moves late, and the TV feed only sends positions from
 * when it joined. Take the export up to the latest position the feed also showed, then the
 * feed's own moves from there. Undefined until the export reaches a position the feed sent.
 * `live[i]` is the move that led from `feed[i]` to `feed[i + 1]`.
 */
export function alignTvMoves(
  setup: GameSetup,
  san: readonly string[],
  feed: readonly string[],
  live: readonly (string | undefined)[],
): string[] | undefined {
  const start = setupStart(setup)
  if (!start || !feed.length) return undefined
  const position = start.clone()
  const exported: string[] = []
  const keys = [positionKey(makeFen(position.toSetup()))]
  for (const token of san) {
    const move = parseSan(position, token)
    if (!move) return undefined
    exported.push(makeUci(move))
    position.play(move)
    keys.push(positionKey(makeFen(position.toSetup())))
  }
  const target = positionKey(feed.at(-1)!)
  for (let ply = keys.length - 1; ply >= 0; ply--) {
    const seen = feed.lastIndexOf(keys[ply]!)
    if (seen < 0) continue
    const since = live.slice(seen)
    if (since.some((move) => !move)) return undefined
    const moves = [...exported.slice(0, ply), ...(since as string[])]
    const replayed = replaySetup(setup, moves)
    if (replayed?.played.length !== moves.length) return undefined
    return positionKey(makeFen(replayed.position.toSetup())) === target ? moves : undefined
  }
  return undefined
}

/** Waits, returning early when the watch is cancelled. */
function pause(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, ms)
    signal.addEventListener(
      'abort',
      () => {
        clearTimeout(timer)
        resolve()
      },
      { once: true },
    )
  })
}
/** What the feed has shown of the featured game, to line its delayed export up with. */
interface TvTrack {
  gameId: string
  feed: string[]
  live: (string | undefined)[]
}
/** How often, and how many times, to ask for the featured game until its export catches up. */
const LINE_UP_DELAY_MS = 5_000
const LINE_UP_ATTEMPTS = 12
const PAST_GAMES = 6

/** One live stream at a time: watching something else stops the last thing. */
export class Spectator {
  private controller: AbortController | null = null
  private session = 0
  /** Earlier games per TV channel, kept for the app's lifetime. */
  private past = new Map<string, TvPastGame[]>()
  constructor(
    private frame: (frame: WatchFrame) => void,
    private broadcast: (update: BroadcastUpdate) => void,
    private state: (state: WatchState) => void = () => {},
  ) {}

  stop(): void {
    this.session++
    this.controller?.abort()
    this.controller = null
  }

  /** Watch a TV channel or a game; returns the session number its frames carry. */
  watch(target: { channel: string } | { gameId: string }): number {
    this.stop()
    const session = this.session
    const controller = new AbortController()
    this.controller = controller
    const signal = controller.signal
    const report = (phase: WatchState['phase'], message?: string): void => {
      if (!signal.aborted && session === this.session) this.state({ session, phase, message })
    }
    queueMicrotask(() => report('connecting'))
    void withUsage('', 'watch', async () => {
      try {
        if ('channel' in target) await this.followTv(target.channel, session, signal)
        else await this.followGame(target.gameId, session, signal)
        report('ended', 'The feed ended. Retry to reconnect.')
      } catch (cause) {
        report(
          'error',
          cause instanceof Error ? cause.message : 'The feed disconnected. Retry to reconnect.',
        )
      }
    })
    return session
  }

  private async followTv(channel: string, session: number, signal: AbortSignal): Promise<void> {
    const stream = await unwrap(
      client.GET('/api/tv/{channel}/feed', {
        params: { path: { channel } },
        parseAs: 'stream',
        signal,
      }),
    )
    let current: WatchFrame | null = null
    let track: TvTrack | null = null
    await readLines(
      stream,
      (line) => {
        if (signal.aborted || session !== this.session) return
        const message = JSON.parse(line) as { t?: string; d?: Record<string, unknown> }
        const data = message.d ?? {}
        if (message.t === 'featured') {
          const players = Array.isArray(data.players) ? data.players : []
          const side = (color: 'white' | 'black') =>
            players.find((p) => (p as { color?: string }).color === color) as
              { seconds?: number } | undefined
          const fen = displayFen(String(data.fen ?? ''))
          if (!fen || typeof data.id !== 'string') return
          const gameId = data.id.slice(0, 12)
          if (current && current.gameId !== gameId) this.remember(channel, current, signal)
          current = {
            session,
            source: 'tv',
            channel,
            gameId,
            white: playerOf(side('white')),
            black: playerOf(side('black')),
            orientation: data.orientation === 'black' ? 'black' : 'white',
            fen,
            whiteClock: side('white')?.seconds,
            blackClock: side('black')?.seconds,
            variant: channel,
            finished: false,
            previous: this.past.get(channel),
          }
          track = { gameId, feed: [positionKey(fen)], live: [] }
          this.state({ session, phase: 'connected' })
          this.frame(current)
          void this.lineUp(
            track,
            signal,
            () => session === this.session,
            (patch) => {
              if (current?.gameId !== gameId) return
              current = { ...current, ...patch }
              this.frame(current)
            },
          )
        } else if (message.t === 'fen' && current && track) {
          const fen = displayFen(String(data.fen ?? ''))
          if (!fen) return
          const lastMove = uciOrUndefined(data.lm)
          track.feed.push(positionKey(fen))
          track.live.push(lastMove)
          current = {
            ...current,
            fen,
            lastMove,
            whiteClock: typeof data.wc === 'number' ? data.wc : current.whiteClock,
            blackClock: typeof data.bc === 'number' ? data.bc : current.blackClock,
            // A move the feed did not name breaks the list; better none than a wrong one.
            moves: current.moves && lastMove ? [...current.moves, lastMove] : undefined,
          }
          this.state({ session, phase: 'connected' })
          this.frame(current)
        }
      },
      { signal },
    )
  }

  /**
   * Fills in what the TV feed leaves out: time control, rated flag and, once Lichess's delayed
   * export reaches a position the feed showed, the moves so far. Failures stay quiet; the board
   * works without them.
   */
  private async lineUp(
    track: TvTrack,
    signal: AbortSignal,
    current: () => boolean,
    apply: (patch: Partial<WatchFrame>) => void,
  ): Promise<void> {
    for (let attempt = 0; attempt < LINE_UP_ATTEMPTS; attempt++) {
      if (attempt) await pause(LINE_UP_DELAY_MS, signal)
      if (signal.aborted || !current()) return
      let game: TvGame
      try {
        game = await fetchTvGame(track.gameId, signal)
      } catch {
        continue
      }
      if (signal.aborted || !current()) return
      const moves = game.setup && alignTvMoves(game.setup, game.san, track.feed, track.live)
      if (attempt === 0 || moves)
        apply({
          ...game.details,
          ...(game.setup ? { variant: game.setup.variant } : {}),
          ...(moves ? { startFen: game.setup!.fen, moves } : {}),
        })
      if (moves || !game.setup) return
    }
  }

  /** Keeps a game the channel moved on from, and fetches how it ended. */
  private remember(channel: string, frame: WatchFrame, signal: AbortSignal): void {
    const entry: TvPastGame = {
      gameId: frame.gameId,
      white: frame.white,
      black: frame.black,
      fen: frame.fen,
      lastMove: frame.lastMove,
      orientation: frame.orientation,
    }
    const list = [
      entry,
      ...(this.past.get(channel) ?? []).filter((game) => game.gameId !== entry.gameId),
    ]
    this.past.set(channel, list.slice(0, PAST_GAMES))
    fetchTvGame(entry.gameId, signal)
      .then((game) => {
        entry.status = game.status
        entry.winner = game.winner
      })
      .catch(() => {})
  }

  private async followGame(id: string, session: number, signal: AbortSignal): Promise<void> {
    const stream = await unwrap(
      client.GET('/api/stream/game/{id}', { params: { path: { id } }, parseAs: 'stream', signal }),
    )
    let current: WatchFrame | null = null
    await readLines(
      stream,
      (line) => {
        if (signal.aborted || session !== this.session) return
        const data = JSON.parse(line) as Record<string, unknown>
        const fen = displayFen(String(data.fen ?? ''))
        if (!fen) return
        if (typeof data.id === 'string') {
          // The description of the game: first, and again when it ends.
          const status = (data.status as { name?: string } | undefined)?.name
          const players = (data.players ?? {}) as { white?: unknown; black?: unknown }
          current = {
            session,
            source: 'game',
            gameId: data.id.slice(0, 12),
            white: playerOf(players.white),
            black: playerOf(players.black),
            orientation: current?.orientation ?? 'white',
            fen,
            lastMove: uciOrUndefined(data.lastMove),
            whiteClock: current?.whiteClock,
            blackClock: current?.blackClock,
            variant: (data.variant as { key?: string } | undefined)?.key ?? 'standard',
            ...parseGameDetails(data),
            status,
            winner: data.winner === 'white' || data.winner === 'black' ? data.winner : undefined,
            finished: Boolean(status && !['created', 'started'].includes(status)),
          }
        } else if (current)
          current = {
            ...current,
            fen,
            lastMove: uciOrUndefined(data.lm),
            whiteClock: typeof data.wc === 'number' ? data.wc : current.whiteClock,
            blackClock: typeof data.bc === 'number' ? data.bc : current.blackClock,
          }
        if (current) {
          this.state({ session, phase: 'connected' })
          this.frame(current)
        }
      },
      { signal },
    )
  }

  /** Follow a broadcast round's live PGN. */
  watchRound(roundId: string): number {
    this.stop()
    const session = this.session
    const controller = new AbortController()
    this.controller = controller
    const signal = controller.signal
    void withUsage('', 'watch', async () => {
      try {
        const stream = await unwrap(
          client.GET('/api/stream/broadcast/round/{broadcastRoundId}.pgn', {
            params: { path: { broadcastRoundId: roundId } },
            parseAs: 'stream',
            signal,
          }),
        )
        await readPgnStream(stream, signal, (games) => {
          if (session === this.session && !signal.aborted && games.length)
            this.broadcast({ session, roundId, games })
        })
        if (session === this.session) this.broadcast({ session, roundId, games: [], ended: true })
      } catch (cause) {
        if (session === this.session && !signal.aborted)
          this.broadcast({
            session,
            roundId,
            games: [],
            ended: true,
            error: cause instanceof Error ? cause.message : 'The broadcast feed stopped.',
          })
      }
    })
    return session
  }
}

const MAX_PGN = 2_000_000

/** Read concatenated PGN games as they stream in, handing over each batch that parsed. */
async function readPgnStream(
  stream: ReadableStream<Uint8Array>,
  signal: AbortSignal,
  onGames: (games: BroadcastGame[]) => void,
): Promise<void> {
  const reader = stream.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  const flush = (final: boolean): void => {
    // Lichess ends every game it sends with two blank lines.
    const parts = buffer.split('\n\n\n')
    buffer = final ? '' : (parts.pop() ?? '')
    const games = parts.flatMap((pgn) => {
      const game = broadcastGame(pgn)
      return game ? [game] : []
    })
    if (games.length) onGames(games)
  }
  try {
    for (;;) {
      if (signal.aborted) return
      const { done, value } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      if (buffer.length > MAX_PGN) buffer = buffer.slice(-MAX_PGN)
      flush(false)
    }
    buffer += decoder.decode()
    flush(true)
  } finally {
    await reader.cancel().catch(() => undefined)
    reader.releaseLock()
  }
}

const clockOf = (comment: string | undefined): number | undefined => {
  if (!comment) return undefined
  const clock = parseComment(comment).clock
  return clock === undefined ? undefined : Math.round(clock)
}

/** One broadcast game from its PGN, with the main line replayed and checked. */
export function broadcastGame(pgn: string): BroadcastGame | undefined {
  const game = parsePgn(pgn.slice(0, 200_000))[0]
  if (!game) return undefined
  const headers = game.headers
  const start = startingPosition(headers)
  if (start.isErr) return undefined
  const pos = start.value
  const startFen = makeFen(pos.toSetup())
  const moves: string[] = []
  let whiteClock: number | undefined
  let blackClock: number | undefined
  for (const node of game.moves.mainline()) {
    const move = parseSan(pos, node.san)
    if (!move) break
    const mover = pos.turn
    pos.play(move)
    moves.push(makeUci(move))
    const clock = clockOf(node.comments?.join(' '))
    if (clock !== undefined) {
      if (mover === 'white') whiteClock = clock
      else blackClock = clock
    }
    if (moves.length > 1000) break
  }
  const chapter =
    /\/([a-zA-Z0-9]{8})$/.exec(headers.get('ChapterURL') ?? '')?.[1] ??
    /\/([a-zA-Z0-9]{8})$/.exec(headers.get('GameURL') ?? '')?.[1] ??
    `${headers.get('White') ?? ''}-${headers.get('Black') ?? ''}-${headers.get('Board') ?? ''}`
  const rating = (value: string | undefined): number | undefined => {
    const n = Number(value)
    return Number.isFinite(n) && n > 0 ? n : undefined
  }
  const result = headers.get('Result') ?? '*'
  return {
    id: chapter.slice(0, 64),
    name: (headers.get('Event') ?? '').slice(0, 120),
    white: {
      name: (headers.get('White') ?? 'White').slice(0, 60),
      title: headers.get('WhiteTitle')?.slice(0, 8),
      rating: rating(headers.get('WhiteElo')),
    },
    black: {
      name: (headers.get('Black') ?? 'Black').slice(0, 60),
      title: headers.get('BlackTitle')?.slice(0, 8),
      rating: rating(headers.get('BlackElo')),
    },
    result,
    startFen,
    moves,
    fen: makeFen(pos.toSetup()),
    lastMove: moves.at(-1),
    whiteClock,
    blackClock,
    ongoing: result === '*',
    pgn: pgn.slice(0, 200_000),
  }
}

/** Lichess TV channels with who is playing on each right now. */
export async function tvChannels(): Promise<TvChannel[]> {
  const raw = (await withUsage('', 'watch', () =>
    unwrap(client.GET('/api/tv/channels')),
  )) as Record<
    string,
    { user?: { name?: string; title?: string }; rating?: number; gameId?: string } | undefined
  >
  return TV_CHANNELS.map((channel) => {
    const entry = raw[channel.key]
    return {
      key: channel.key,
      label: channel.label,
      gameId: typeof entry?.gameId === 'string' ? entry.gameId : undefined,
      player: entry?.user?.name
        ? { name: entry.user.name, title: entry.user.title ?? undefined, rating: entry.rating }
        : undefined,
    }
  })
}

const roundSchema = v.looseObject({
  id: text(12),
  name: text(120),
  ongoing: v.optional(v.boolean()),
  finished: v.optional(v.boolean()),
  finishedAt: v.optional(num),
  startsAt: v.optional(num),
})
const tourSchema = v.looseObject({
  info: v.optional(v.looseObject({ players: v.optional(text(500)) })),
  id: text(12),
  name: text(200),
  description: v.optional(text(4000)),
  image: v.optional(text(400)),
})
const entrySchema = v.looseObject({ tour: v.optional(tourSchema), round: v.optional(roundSchema) })

/** The broadcasts Lichess features: live, upcoming, then recently finished. */
export async function broadcasts(query?: string): Promise<BroadcastSummary[]> {
  if (query?.trim()) {
    const raw = await withUsage('', 'watch', () =>
      unwrap(
        client.GET('/api/broadcast/search', { params: { query: { q: query.trim(), page: 1 } } }),
      ),
    )
    return broadcastSummaries(raw.currentPageResults, 'past')
  }
  const raw = await withUsage('', 'watch', () =>
    unwrap(client.GET('/api/broadcast/top', { params: { query: { page: 1 } } })),
  )
  const sections: [BroadcastSummary['section'], unknown[] | undefined][] = [
    ['active', raw.active],
    ['upcoming', raw.upcoming],
    ['past', raw.past?.currentPageResults],
  ]
  return sections.flatMap(([section, entries]) => broadcastSummaries(entries, section))
}

/** Normalize both featured and search entries; only trust Lichess image URLs. */
export function broadcastSummaries(
  entries: unknown[] | undefined,
  section: BroadcastSummary['section'],
): BroadcastSummary[] {
  return (entries ?? []).slice(0, 30).flatMap((entry) => {
    const parsed = v.safeParse(entrySchema, entry)
    if (!parsed.success || !parsed.output.tour) return []
    const { tour, round } = parsed.output
    return [
      {
        players: tour.info?.players,
        tourId: tour.id,
        tourName: tour.name,
        description: tour.description,
        image: tour.image?.startsWith('https://image.lichess1.org/') ? tour.image : undefined,
        roundId: round?.id,
        roundName: round?.name,
        ongoing: Boolean(round?.ongoing),
        startsAt: round?.startsAt,
        section: round?.ongoing
          ? 'active'
          : round && !round.finished && !round.finishedAt
            ? 'upcoming'
            : section,
      },
    ]
  })
}

export async function broadcastTour(id: string): Promise<BroadcastTourDetail> {
  const raw = (await withUsage('', 'watch', () =>
    unwrap(
      client.GET('/api/broadcast/{broadcastTournamentId}', {
        params: { path: { broadcastTournamentId: id } },
      }),
    ),
  )) as { tour?: unknown; rounds?: unknown[]; defaultRoundId?: string }
  const tour = v.parse(tourSchema, raw.tour)
  return {
    id: tour.id,
    name: tour.name,
    description: tour.description,
    rounds: (raw.rounds ?? []).slice(0, 100).flatMap((round) => {
      const parsed = v.safeParse(roundSchema, round)
      return parsed.success
        ? [
            {
              id: parsed.output.id,
              name: parsed.output.name,
              ongoing: Boolean(parsed.output.ongoing),
              finished: Boolean(parsed.output.finished),
              startsAt: parsed.output.startsAt,
            },
          ]
        : []
    }),
    defaultRoundId: typeof raw.defaultRoundId === 'string' ? raw.defaultRoundId : undefined,
  }
}
