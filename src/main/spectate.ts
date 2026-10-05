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
  WatchFrame,
  WatchPlayer,
} from '../shared/types'
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

/** One live stream at a time: watching something else stops the last thing. */
export class Spectator {
  private controller: AbortController | null = null
  private session = 0
  constructor(
    private frame: (frame: WatchFrame) => void,
    private broadcast: (update: BroadcastUpdate) => void,
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
    void withUsage('', 'watch', async () => {
      try {
        if ('channel' in target) await this.followTv(target.channel, session, signal)
        else await this.followGame(target.gameId, session, signal)
      } catch {
        // A dropped feed just stops; the window offers to watch again.
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
          current = {
            session,
            source: 'tv',
            channel,
            gameId: data.id.slice(0, 12),
            white: playerOf(side('white')),
            black: playerOf(side('black')),
            orientation: data.orientation === 'black' ? 'black' : 'white',
            fen,
            whiteClock: side('white')?.seconds,
            blackClock: side('black')?.seconds,
            variant: channel,
            finished: false,
          }
          this.frame(current)
        } else if (message.t === 'fen' && current) {
          const fen = displayFen(String(data.fen ?? ''))
          if (!fen) return
          current = {
            ...current,
            fen,
            lastMove: uciOrUndefined(data.lm),
            whiteClock: typeof data.wc === 'number' ? data.wc : current.whiteClock,
            blackClock: typeof data.bc === 'number' ? data.bc : current.blackClock,
          }
          this.frame(current)
        }
      },
      { signal },
    )
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
            speed: typeof data.speed === 'string' ? data.speed : undefined,
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
        if (current) this.frame(current)
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
  startsAt: v.optional(num),
})
const tourSchema = v.looseObject({
  id: text(12),
  name: text(200),
  description: v.optional(text(4000)),
  image: v.optional(text(400)),
})
const entrySchema = v.looseObject({ tour: v.optional(tourSchema), round: v.optional(roundSchema) })

/** The broadcasts Lichess features: live, upcoming, then recently finished. */
export async function broadcasts(): Promise<BroadcastSummary[]> {
  const raw = await withUsage('', 'watch', () =>
    unwrap(client.GET('/api/broadcast/top', { params: { query: { page: 1 } } })),
  )
  const sections: [BroadcastSummary['section'], unknown[] | undefined][] = [
    ['active', raw.active],
    ['upcoming', raw.upcoming],
    ['past', raw.past?.currentPageResults],
  ]
  return sections.flatMap(([section, entries]) =>
    (entries ?? []).slice(0, 30).flatMap((entry) => {
      const parsed = v.safeParse(entrySchema, entry)
      if (!parsed.success || !parsed.output.tour) return []
      const { tour, round } = parsed.output
      return [
        {
          tourId: tour.id,
          tourName: tour.name,
          description: tour.description,
          // Images come from Lichess's own CDN; anything else is dropped.
          image: tour.image?.startsWith('https://image.lichess1.org/') ? tour.image : undefined,
          roundId: round?.id,
          roundName: round?.name,
          ongoing: Boolean(round?.ongoing),
          startsAt: round?.startsAt,
          section,
        },
      ]
    }),
  )
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
