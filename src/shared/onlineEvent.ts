import * as v from 'valibot'
import type { OnlineEvent } from './types'
import { GAME_ID, UCI_MOVE } from './patterns'

const id = v.pipe(v.string(), v.regex(GAME_ID))
const clock = v.pipe(v.number(), v.finite(), v.minValue(0))
const state = v.looseObject({
  type: v.literal('gameState'),
  moves: v.pipe(v.string(), v.maxLength(10_000)),
  status: v.string(),
  wtime: v.optional(clock),
  btime: v.optional(clock),
})
const game = v.looseObject({
  gameId: id,
  color: v.optional(v.picklist(['white', 'black'])),
  opponent: v.optional(
    v.looseObject({ username: v.optional(v.string()), id: v.optional(v.string()) }),
  ),
})
const full = v.looseObject({
  type: v.literal('gameFull'),
  id,
  white: v.looseObject({ id: v.optional(v.string()), name: v.optional(v.string()) }),
  black: v.looseObject({ id: v.optional(v.string()), name: v.optional(v.string()) }),
  state: v.looseObject({
    moves: v.pipe(v.string(), v.maxLength(10_000)),
    status: v.string(),
    wtime: v.optional(clock),
    btime: v.optional(clock),
  }),
})
const schema = v.variant('type', [
  state,
  full,
  v.looseObject({ type: v.literal('gameStart'), game }),
  v.looseObject({ type: v.literal('gameFinish'), game }),
  v.looseObject({
    type: v.literal('opponentGone'),
    gone: v.boolean(),
    claimWinInSeconds: v.optional(clock),
  }),
  v.looseObject({
    type: v.literal('chatLine'),
    username: v.string(),
    text: v.string(),
    room: v.string(),
  }),
  v.looseObject({ type: v.literal('challenge'), challenge: v.looseObject({ id }) }),
  v.looseObject({ type: v.literal('challengeCanceled'), challenge: v.looseObject({ id }) }),
  v.looseObject({ type: v.literal('challengeDeclined'), challenge: v.looseObject({ id }) }),
])
/** Unknown event types can be ignored; malformed recognized game state is a recoverable error. */
export function validateOnlineEvent(raw: unknown): OnlineEvent | undefined {
  if (!raw || typeof raw !== 'object' || !('type' in raw)) return undefined
  const types = [
    'gameState',
    'gameFull',
    'gameStart',
    'gameFinish',
    'opponentGone',
    'chatLine',
    'challenge',
    'challengeCanceled',
    'challengeDeclined',
  ]
  if (!types.includes(String(raw.type))) return undefined
  const result = v.safeParse(schema, raw)
  if (!result.success)
    throw new Error('Lichess sent an unreadable game event. Reconnect to refresh the board.')
  const event = result.output
  const moves =
    event.type === 'gameState'
      ? event.moves
      : event.type === 'gameFull'
        ? event.state.moves
        : undefined
  if (
    moves &&
    !moves
      .trim()
      .split(/\s+/)
      .every((move) => UCI_MOVE.test(move))
  )
    throw new Error('Lichess sent an invalid move list.')
  return event as OnlineEvent
}
