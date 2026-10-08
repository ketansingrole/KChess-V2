import * as v from 'valibot'
import type { ChallengeInfo, ChallengeColor } from '../shared/types'
import { GAME_ID } from '../shared/patterns'
import { variantFromLichess } from '../shared/variant'

const text = (max: number) => v.pipe(v.string(), v.maxLength(max))
const user = v.looseObject({
  id: text(40),
  name: text(40),
  rating: v.optional(v.pipe(v.number(), v.finite())),
  title: v.optional(v.nullable(text(8))),
  provisional: v.optional(v.boolean()),
  online: v.optional(v.boolean()),
})
const challengeSchema = v.looseObject({
  id: v.pipe(v.string(), v.regex(GAME_ID)),
  status: v.optional(text(20)),
  challenger: v.nullable(user),
  destUser: v.nullable(user),
  variant: v.looseObject({ key: text(30), name: v.optional(text(40)) }),
  rated: v.boolean(),
  speed: text(20),
  timeControl: v.variant('type', [
    v.looseObject({
      type: v.literal('clock'),
      limit: v.optional(v.pipe(v.number(), v.finite(), v.minValue(0))),
      increment: v.optional(v.pipe(v.number(), v.finite(), v.minValue(0))),
    }),
    v.looseObject({
      type: v.literal('correspondence'),
      daysPerTurn: v.optional(v.pipe(v.number(), v.finite(), v.minValue(0))),
    }),
    v.looseObject({ type: v.literal('unlimited') }),
  ]),
  color: v.optional(v.picklist(['white', 'black', 'random'])),
  direction: v.optional(v.picklist(['in', 'out'])),
  initialFen: v.optional(text(100)),
  rematchOf: v.optional(v.pipe(v.string(), v.regex(GAME_ID))),
})
const eventSchema = v.looseObject({
  type: v.picklist(['challenge', 'challengeCanceled', 'challengeDeclined']),
  challenge: challengeSchema,
  compat: v.optional(v.looseObject({ board: v.optional(v.boolean()) })),
})

/** Real-time challenges Lichess no longer reports are forgotten after this long. */
const LIVE_TTL_MS = 30 * 60_000
const CORRESPONDENCE_TTL_MS = 14 * 86_400_000
const MAX_CHALLENGES = 50

/** Read a challenge event from the event stream of `account`; undefined when it is not one. */
export function readChallengeEvent(
  account: string,
  raw: unknown,
  now = Date.now(),
):
  | { type: 'challenge' | 'challengeCanceled' | 'challengeDeclined'; info: ChallengeInfo }
  | undefined {
  const parsed = v.safeParse(eventSchema, raw)
  if (!parsed.success) return undefined
  const { challenge, compat } = parsed.output
  const ours = account.toLowerCase()
  const outgoing =
    challenge.direction === 'out' ||
    (challenge.direction === undefined && challenge.challenger?.id.toLowerCase() === ours)
  const other = outgoing ? challenge.destUser : challenge.challenger
  const variant = variantFromLichess(challenge.variant.key)
  const control = challenge.timeControl
  const boardOk = compat?.board !== false
  const problem = !variant
    ? `${challenge.variant.name ?? challenge.variant.key} needs a piece-drop board KChess does not have yet.`
    : !boardOk
      ? `Lichess does not let third-party apps play ${challenge.speed} games.`
      : undefined
  return {
    type: parsed.output.type,
    info: {
      id: challenge.id,
      account,
      direction: outgoing ? 'out' : 'in',
      opponent: {
        name: other?.name ?? 'Anyone',
        rating: other?.rating,
        title: other?.title ?? undefined,
        provisional: other?.provisional,
        online: other?.online,
      },
      variant: challenge.variant.key,
      variantName: challenge.variant.name ?? challenge.variant.key,
      rated: challenge.rated,
      speed: challenge.speed,
      timeControl:
        control.type === 'clock'
          ? { type: 'clock', limit: control.limit ?? 0, increment: control.increment ?? 0 }
          : control.type === 'correspondence'
            ? { type: 'correspondence', days: control.daysPerTurn ?? 0 }
            : { type: 'unlimited' },
      color: (challenge.color ?? 'random') as ChallengeColor,
      rematchOf: challenge.rematchOf,
      initialFen:
        challenge.initialFen && challenge.initialFen !== 'startpos'
          ? challenge.initialFen
          : undefined,
      playable: !problem,
      problem,
      receivedAt: now,
    },
  }
}

/** Pending challenges of every connected account, fed by whichever event stream is open. */
export class ChallengeInbox {
  private entries = new Map<string, ChallengeInfo>()
  constructor(
    private changed: (list: ChallengeInfo[]) => void,
    private now = Date.now,
  ) {}

  list(): ChallengeInfo[] {
    this.prune()
    return [...this.entries.values()].sort((a, b) => b.receivedAt - a.receivedAt)
  }

  get(id: string): ChallengeInfo | undefined {
    this.prune()
    return this.entries.get(id)
  }

  /** Apply one event-stream event; returns the challenge when it is new and incoming. */
  ingest(account: string, raw: unknown): ChallengeInfo | undefined {
    const event = readChallengeEvent(account, raw, this.now())
    if (!event) return undefined
    const known = this.entries.get(event.info.id)
    if (event.type === 'challenge') {
      // Lichess repeats pending challenges whenever the stream reconnects; keep the first sighting.
      this.entries.set(
        event.info.id,
        known ? { ...event.info, receivedAt: known.receivedAt } : event.info,
      )
      while (this.entries.size > MAX_CHALLENGES)
        this.entries.delete(this.entries.keys().next().value!)
    } else this.entries.delete(event.info.id)
    this.emit()
    return event.type === 'challenge' && !known && event.info.direction === 'in'
      ? event.info
      : undefined
  }

  remove(id: string): void {
    if (this.entries.delete(id)) this.emit()
  }

  /** Forget an account's challenges (it was disconnected). */
  forget(account: string): void {
    let removed = false
    for (const [id, info] of this.entries)
      if (info.account.toLowerCase() === account.toLowerCase()) {
        this.entries.delete(id)
        removed = true
      }
    if (removed) this.emit()
  }

  private prune(): void {
    const now = this.now()
    for (const [id, info] of this.entries) {
      const ttl = info.timeControl.type === 'clock' ? LIVE_TTL_MS : CORRESPONDENCE_TTL_MS
      if (now - info.receivedAt > ttl) this.entries.delete(id)
    }
  }

  private emit(): void {
    this.changed(this.list())
  }
}
