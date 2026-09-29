/** Input validation for values crossing the renderer → main IPC boundary. */
import * as v from 'valibot'
import {
  APPEARANCES,
  CHALLENGE_COLORS,
  COORDINATE_MODES,
  ENGINE_LEVELS,
  ONLINE_ACTIONS,
  PROMOTION_MODES,
  type EngineLevel,
  type OnlineAction,
  type OnlineOptions,
  type Settings,
} from './types.ts'
import { GAME_ID, UCI_MOVE, USERNAME } from './patterns.ts'

export { GAME_ID, UCI_MOVE, USERNAME }
export { ENGINE_LEVELS, ONLINE_ACTIONS }
export type { EngineLevel, OnlineAction }

const MAX_MOVES = 1024
const USERNAME_MESSAGE = 'Enter a valid Lichess username.'

const usernameSchema = v.pipe(
  v.string(USERNAME_MESSAGE),
  v.trim(),
  v.regex(USERNAME, USERNAME_MESSAGE),
)
const gameIdSchema = v.pipe(v.string('Invalid game id.'), v.regex(GAME_ID, 'Invalid game id.'))
const uciSchema = v.pipe(v.string('Invalid move.'), v.regex(UCI_MOVE, 'Invalid move.'))
const movesSchema = v.pipe(
  v.array(uciSchema, 'Invalid move list.'),
  v.maxLength(MAX_MOVES, 'Invalid move list.'),
)
const levelSchema = v.picklist(ENGINE_LEVELS, 'Invalid engine level.')
const usernamesSchema = v.pipe(
  v.array(usernameSchema, 'Invalid users.'),
  v.maxLength(50, 'Invalid users.'),
)
const friendListSchema = v.pipe(
  v.array(usernameSchema, 'Invalid users.'),
  v.minLength(1, 'Choose at least one player.'),
  v.maxLength(1000, 'Invalid users.'),
)
const actionSchema = v.picklist(ONLINE_ACTIONS, 'Invalid game action.')

const clockField = (min: number, message: string) =>
  v.pipe(v.number(message), v.integer(message), v.minValue(min, message), v.maxValue(180, message))

const onlineOptionsSchema = v.object(
  {
    minutes: clockField(1, 'Invalid time control.'),
    increment: clockField(0, 'Invalid increment.'),
    color: v.picklist(CHALLENGE_COLORS, 'Invalid color.'),
    rated: v.boolean('Invalid rated option.'),
    account: v.optional(usernameSchema),
    target: v.optional(
      v.pipe(
        v.string('Invalid opponent.'),
        v.trim(),
        v.check((s) => s === '' || USERNAME.test(s), USERNAME_MESSAGE),
        v.transform((s) => s || undefined),
      ),
    ),
  },
  'Invalid game options.',
)

const settingsSchema = v.object(
  {
    appearance: v.picklist(APPEARANCES, 'Invalid appearance.'),
    boardTheme: v.pipe(
      v.string('Invalid board theme.'),
      v.regex(/^[a-z0-9-]{1,32}$/, 'Invalid board theme.'),
    ),
    coordinates: v.picklist(COORDINATE_MODES, 'Invalid coordinates mode.'),
    soundEnabled: v.boolean('Invalid sound setting.'),
    soundVolume: v.pipe(
      v.number('Invalid sound volume.'),
      v.finite('Invalid sound volume.'),
      v.transform((n) => Math.max(0, Math.min(1, n))),
    ),
    enginePath: v.pipe(v.string('Invalid engine path.'), v.maxLength(4096, 'Invalid engine path.')),
    premove: v.boolean('Invalid premove setting.'),
    promotion: v.picklist(PROMOTION_MODES, 'Invalid promotion setting.'),
    showLegalMoves: v.boolean('Invalid legal moves setting.'),
  },
  'Invalid settings.',
)

/** Parse `value` or throw an Error carrying the schema's UI-safe message. */
function parse<const S extends v.GenericSchema>(schema: S, value: unknown): v.InferOutput<S> {
  const result = v.safeParse(schema, value)
  if (!result.success) throw new Error(result.issues[0].message)
  return result.output
}

export const assertUsername = (value: unknown): string => parse(usernameSchema, value)
export const assertGameId = (value: unknown): string => parse(gameIdSchema, value)
export const assertUci = (value: unknown): string => parse(uciSchema, value)
export const assertMoves = (value: unknown): string[] => parse(movesSchema, value)
export const assertLevel = (value: unknown): EngineLevel => parse(levelSchema, value)
export const assertUsernames = (value: unknown): string[] => parse(usernamesSchema, value)
export const assertFriendList = (value: unknown): string[] => parse(friendListSchema, value)
export const assertAction = (value: unknown): OnlineAction => parse(actionSchema, value)
export const assertOnlineOptions = (value: unknown): OnlineOptions =>
  parse(onlineOptionsSchema, value)
export const assertSettings = (value: unknown): Settings => parse(settingsSchema, value)
