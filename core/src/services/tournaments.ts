import * as v from 'valibot'
import type {
  NeedsReconnect,
  NewArena,
  TournamentDetail,
  TournamentList,
  TournamentStanding,
  TournamentSummary,
  TournamentSystem,
} from '../contracts/types'
import { canBoardSeek } from '../domain/timeControl'
import { LichessError } from '../domain/lichessError'
import { variantFromLichess } from '../domain/variant'
import { asAccount, asOwner, authorize, client, unwrap, urlencoded } from './lichess'
import { logDebug, logWarn } from './logger'
import { readLines } from './ndjson'
import { withUsage } from './usage'
import { getToken } from './store'

const text = (max: number) => v.pipe(v.string(), v.maxLength(max))
const num = v.pipe(v.number(), v.finite())
const variantField = v.union([
  text(40),
  v.looseObject({ key: text(40), name: v.optional(text(40)) }),
])
const clockSchema = v.looseObject({ limit: num, increment: num })

const STATUS: Record<number, TournamentSummary['status']> = {
  10: 'created',
  20: 'started',
  30: 'finished',
}

function variantOf(raw: v.InferOutput<typeof variantField>): { key: string; name: string } {
  return typeof raw === 'string'
    ? { key: raw, name: raw }
    : { key: raw.key, name: raw.name ?? raw.key }
}

/** Whether KChess can play this tournament's games through the Board API. */
function compatibility(
  clock: { limit: number; increment: number },
  variant: string,
): Pick<TournamentSummary, 'playable' | 'problem'> {
  if (!variantFromLichess(variant))
    return { playable: false, problem: `KChess cannot show ${variant} games yet.` }
  if (!canBoardSeek(clock.limit / 60, clock.increment))
    return {
      playable: false,
      problem:
        'Lichess lets third-party apps play only Rapid and Classical tournament games (this is faster).',
    }
  return { playable: true }
}

const arenaSchema = v.looseObject({
  id: text(12),
  fullName: text(120),
  status: num,
  variant: variantField,
  rated: v.boolean(),
  clock: clockSchema,
  minutes: v.optional(num),
  nbPlayers: num,
  startsAt: num,
  finishesAt: v.optional(num),
  perf: v.optional(v.looseObject({ key: text(30) })),
  schedule: v.optional(v.nullable(v.looseObject({ freq: v.optional(text(20)) }))),
  maxRating: v.optional(v.nullable(v.looseObject({ rating: num }))),
})

function arenaSummary(raw: v.InferOutput<typeof arenaSchema>): TournamentSummary {
  const variant = variantOf(raw.variant)
  return {
    id: raw.id,
    system: 'arena',
    name: raw.fullName,
    status: STATUS[raw.status] ?? 'created',
    variant: variant.key,
    variantName: variant.name,
    rated: raw.rated,
    clock: raw.clock,
    minutes: raw.minutes,
    nbPlayers: raw.nbPlayers,
    startsAt: raw.startsAt,
    finishesAt: raw.finishesAt,
    perf: raw.perf?.key,
    freq: raw.schedule?.freq ?? undefined,
    maxRating: raw.maxRating?.rating,
    ...compatibility(raw.clock, variant.key),
  }
}

const swissSchema = v.looseObject({
  id: text(12),
  name: text(120),
  status: v.picklist(['created', 'started', 'finished']),
  variant: variantField,
  rated: v.boolean(),
  clock: clockSchema,
  round: v.optional(num),
  nbRounds: v.optional(num),
  nbPlayers: num,
  startsAt: v.union([text(40), num]),
  nextRound: v.optional(v.looseObject({ in: v.optional(num) })),
  verdicts: v.optional(
    v.looseObject({
      accepted: v.boolean(),
      list: v.array(v.looseObject({ condition: text(200), verdict: text(200) })),
    }),
  ),
})

function swissSummary(
  raw: v.InferOutput<typeof swissSchema>,
  team?: { id: string; name: string },
): TournamentSummary {
  const variant = variantOf(raw.variant)
  return {
    id: raw.id,
    system: 'swiss',
    name: raw.name,
    status: raw.status,
    variant: variant.key,
    variantName: variant.name,
    rated: raw.rated,
    clock: raw.clock,
    round: raw.round,
    nbRounds: raw.nbRounds,
    nbPlayers: raw.nbPlayers,
    startsAt: typeof raw.startsAt === 'number' ? raw.startsAt : Date.parse(raw.startsAt) || 0,
    team,
    ...compatibility(raw.clock, variant.key),
  }
}

/** Arenas Lichess lists now, and the upcoming Swiss events of the account's teams. */
/** A short reason for the Swiss list, never Lichess's raw response. */
function teamProblem(cause: unknown): string {
  if (cause instanceof LichessError && (cause.status === 401 || cause.status === 403))
    return '. Connect the account again to allow it.'
  if (cause instanceof LichessError && cause.status === 429) return '. Lichess asked to slow down.'
  return '.'
}

export async function tournaments(account: string): Promise<TournamentList> {
  const problems: string[] = []
  const arenas = await withUsage(account, 'tournament', async () => {
    const raw = await unwrap(client.GET('/api/tournament'))
    // Recently finished arenas stay on the timeline, faded, for context.
    return [...(raw.started ?? []), ...(raw.created ?? []), ...(raw.finished ?? [])].flatMap(
      (entry) => {
        const parsed = v.safeParse(arenaSchema, entry)
        return parsed.success ? [arenaSummary(parsed.output)] : []
      },
    )
  })
  const swiss: TournamentSummary[] = []
  if (account) {
    try {
      // Lichess lists a player's teams only to signed-in apps.
      const teams = await withUsage(account, 'tournament', () =>
        asOwner(account, (auth) =>
          unwrap(
            client.GET('/api/team/of/{username}', {
              params: { path: { username: account } },
              headers: auth,
            }),
          ),
        ),
      )
      // Lichess asks for one request at a time; a few teams are plenty.
      for (const team of (teams as { id: string; name: string }[]).slice(0, 8)) {
        try {
          const stream = await withUsage(account, 'tournament', () =>
            unwrap(
              client.GET('/api/team/{teamId}/swiss', {
                params: { path: { teamId: team.id }, query: { max: 10 } },
                headers: { Accept: 'application/x-ndjson' },
                parseAs: 'stream',
              }),
            ),
          )
          await readLines(stream, (line) => {
            const parsed = v.safeParse(swissSchema, JSON.parse(line))
            if (parsed.success && parsed.output.status !== 'finished')
              swiss.push(swissSummary(parsed.output, { id: team.id, name: team.name }))
          })
        } catch (cause) {
          logWarn('tournaments', 'Could not load Swiss events:', team.id, cause)
          problems.push(`Couldn’t load Swiss events of ${team.name}${teamProblem(cause)}`)
        }
      }
    } catch (cause) {
      logWarn('tournaments', 'Could not load teams:', account, cause)
      problems.push(`Couldn’t load the teams of @${account}${teamProblem(cause)}`)
    }
  }
  return {
    arenas: arenas.sort((a, b) => a.startsAt - b.startsAt),
    swiss: swiss.sort((a, b) => a.startsAt - b.startsAt),
    problems,
  }
}

const standingSchema = v.looseObject({
  rank: num,
  name: v.optional(text(40)),
  username: v.optional(text(40)),
  title: v.optional(v.nullable(text(8))),
  rating: v.optional(num),
  score: v.optional(num),
  points: v.optional(num),
  sheet: v.optional(
    v.looseObject({ scores: v.optional(text(400)), fire: v.optional(v.boolean()) }),
  ),
})
function standingOf(raw: unknown): TournamentStanding | undefined {
  const parsed = v.safeParse(standingSchema, raw)
  if (!parsed.success) return undefined
  const row = parsed.output
  return {
    rank: row.rank,
    name: row.name ?? row.username ?? '?',
    title: row.title ?? undefined,
    rating: row.rating,
    score: row.score ?? row.points,
    sheet: row.sheet?.scores,
    fire: row.sheet?.fire,
  }
}

const rankedPlayer = v.looseObject({
  name: text(40),
  rating: v.optional(num),
  rank: v.optional(num),
})
const extrasSchema = v.looseObject({
  featured: v.optional(
    v.nullable(
      v.looseObject({
        id: text(12),
        fen: text(100),
        orientation: v.optional(v.picklist(['white', 'black'])),
        lastMove: v.optional(text(8)),
        white: rankedPlayer,
        black: rankedPlayer,
        c: v.optional(v.looseObject({ white: num, black: num })),
      }),
    ),
  ),
  duels: v.optional(
    v.array(
      v.looseObject({
        id: text(12),
        p: v.tuple([
          v.looseObject({ n: text(40), r: v.optional(num), k: v.optional(num) }),
          v.looseObject({ n: text(40), r: v.optional(num), k: v.optional(num) }),
        ]),
      }),
    ),
  ),
  podium: v.optional(
    v.nullable(
      v.array(
        v.looseObject({
          name: text(40),
          rank: num,
          rating: v.optional(num),
          score: v.optional(num),
          performance: v.optional(num),
        }),
      ),
    ),
  ),
  stats: v.optional(
    v.nullable(
      v.looseObject({
        games: num,
        whiteWins: num,
        blackWins: num,
        draws: num,
        berserks: num,
        averageRating: num,
      }),
    ),
  ),
})
const playerOf = (p: v.InferOutput<typeof rankedPlayer>) => ({
  name: p.name,
  rating: p.rating,
  rank: p.rank,
})
/** The featured game, games in progress, podium and totals of an arena; parts that fail are left out. */
export function arenaExtras(raw: unknown): Partial<TournamentDetail> {
  const parsed = v.safeParse(extrasSchema, raw)
  if (!parsed.success) return {}
  const { featured, duels, podium, stats } = parsed.output
  return {
    featured: featured
      ? {
          id: featured.id,
          // Lichess sends the board and side to move only.
          fen: `${featured.fen.split(' ').slice(0, 2).join(' ')} - - 0 1`,
          orientation: featured.orientation ?? 'white',
          lastMove: featured.lastMove,
          white: playerOf(featured.white),
          black: playerOf(featured.black),
          clocks: featured.c,
        }
      : undefined,
    duels: duels?.slice(0, 30).map((duel) => ({
      id: duel.id,
      white: { name: duel.p[0].n, rating: duel.p[0].r, rank: duel.p[0].k },
      black: { name: duel.p[1].n, rating: duel.p[1].r, rank: duel.p[1].k },
    })),
    podium: podium?.slice(0, 3).map((entry) => ({
      name: entry.name,
      rank: entry.rank,
      rating: entry.rating,
      score: entry.score,
      performance: entry.performance,
    })),
    stats: stats ?? undefined,
  }
}

/** One tournament with its leaderboard; as `account` it also says whether you are in it. */
export async function tournament(
  system: TournamentSystem,
  id: string,
  account: string,
  page = 1,
): Promise<TournamentDetail> {
  const token = account
    ? await getToken(account).catch((cause: unknown) => {
        logDebug('tournaments', 'Stored login is unavailable:', account, cause)
        return null
      })
    : null
  const headers = token ? authorize(token) : {}
  return withUsage(account, 'tournament', async () => {
    if (system === 'arena') {
      const raw = (await unwrap(
        client.GET('/api/tournament/{id}', {
          params: { path: { id }, query: { page } },
          headers,
        }),
      )) as Record<string, unknown>
      const clock = v.parse(clockSchema, raw.clock)
      const variantKey = typeof raw.variant === 'string' ? raw.variant : 'standard'
      const status = raw.isFinished ? 'finished' : raw.secondsToStart ? 'created' : 'started'
      const me = raw.me as { rank?: unknown; withdraw?: unknown; gameId?: unknown } | undefined
      const standing = (raw.standing as { players?: unknown[] } | undefined)?.players ?? []
      const verdicts = raw.verdicts as TournamentDetail['verdicts'] | undefined
      return {
        id,
        system,
        name: String(raw.fullName ?? id).slice(0, 120),
        status,
        variant: variantKey,
        variantName: variantKey,
        rated: raw.rated !== false,
        clock,
        minutes: typeof raw.minutes === 'number' ? raw.minutes : undefined,
        nbPlayers: Number(raw.nbPlayers ?? 0),
        startsAt: Date.parse(String(raw.startsAt ?? '')) || 0,
        description:
          typeof raw.description === 'string' ? raw.description.slice(0, 2000) : undefined,
        secondsToStart: typeof raw.secondsToStart === 'number' ? raw.secondsToStart : undefined,
        secondsToFinish: typeof raw.secondsToFinish === 'number' ? raw.secondsToFinish : undefined,
        berserkable: raw.berserkable === true,
        standing: standing.flatMap((row) => standingOf(row) ?? []).slice(0, 50),
        standingPage: page,
        ...arenaExtras(raw),
        me: me
          ? {
              rank: typeof me.rank === 'number' ? me.rank : undefined,
              withdraw: me.withdraw === true,
              gameId: typeof me.gameId === 'string' ? me.gameId : undefined,
            }
          : undefined,
        verdicts:
          verdicts && Array.isArray(verdicts.list)
            ? {
                accepted: Boolean(verdicts.accepted),
                list: verdicts.list.slice(0, 10).map((entry) => ({
                  condition: String(entry.condition).slice(0, 200),
                  verdict: String(entry.verdict).slice(0, 200),
                })),
              }
            : undefined,
        ...compatibility(clock, variantKey),
      }
    }
    const raw = v.parse(
      swissSchema,
      await unwrap(client.GET('/api/swiss/{id}', { params: { path: { id } }, headers })),
    )
    const standing: TournamentStanding[] = []
    const stream = await unwrap(
      client.GET('/api/swiss/{id}/results', {
        params: { path: { id }, query: { nb: 50 } },
        headers: { ...headers, Accept: 'application/x-ndjson' },
        parseAs: 'stream',
      }),
    )
    await readLines(stream, (line) => {
      const row = standingOf(JSON.parse(line))
      if (row) standing.push(row)
    })
    return {
      ...swissSummary(raw),
      standing,
      verdicts: raw.verdicts,
      nextRoundIn: raw.nextRound?.in,
    }
  })
}

export function joinTournament(
  system: TournamentSystem,
  id: string,
  account: string,
  password?: string,
): Promise<true | NeedsReconnect> {
  return asAccount(
    account,
    async (token) => {
      const options = {
        params: { path: { id } },
        body: {
          password: password || undefined,
          pairMeAsap: system === 'arena' ? true : undefined,
        },
        bodySerializer: urlencoded,
        headers: { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' },
      }
      if (system === 'arena')
        await unwrap(client.POST('/api/tournament/{id}/join', options as never))
      else await unwrap(client.POST('/api/swiss/{id}/join', options as never))
      return true as const
    },
    'tournament',
  )
}

export function leaveTournament(
  system: TournamentSystem,
  id: string,
  account: string,
): Promise<true | NeedsReconnect> {
  return asAccount(
    account,
    async (token) => {
      const options = { params: { path: { id } }, headers: authorize(token) }
      if (system === 'arena') await unwrap(client.POST('/api/tournament/{id}/withdraw', options))
      else await unwrap(client.POST('/api/swiss/{id}/withdraw', options))
      return true as const
    },
    'tournament',
  )
}

/** Creates an arena run by `account`; logins made before tournaments were requested must reconnect. */
export function createTournament(
  account: string,
  arena: NewArena,
): Promise<TournamentSummary | NeedsReconnect> {
  return asAccount(
    account,
    async (token) => {
      const raw = await unwrap(
        client.POST('/api/tournament', {
          body: {
            name: arena.name || undefined,
            clockTime: arena.clockTime,
            clockIncrement: arena.clockIncrement,
            minutes: arena.minutes,
            waitMinutes: arena.startDate ? undefined : arena.waitMinutes,
            startDate: arena.startDate,
            variant: arena.variant,
            rated: arena.rated,
            password: arena.password || undefined,
            description: arena.description || undefined,
          },
          bodySerializer: urlencoded,
          headers: { ...authorize(token), 'Content-Type': 'application/x-www-form-urlencoded' },
        } as never),
      )
      const parsed = v.safeParse(arenaSchema, raw)
      if (!parsed.success)
        throw new Error('Lichess created the arena but sent details KChess could not read.')
      return arenaSummary(parsed.output)
    },
    'tournament',
  )
}
