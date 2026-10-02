/** Input validation for values crossing the renderer → main IPC boundary. */
import * as v from 'valibot'
import {
  APPEARANCES,
  CHALLENGE_COLORS,
  COORDINATE_MODES,
  ENGINE_LEVELS,
  NOTIFICATION_KINDS,
  ONLINE_ACTIONS,
  PIECE_ANIMATIONS,
  PROMOTION_MODES,
  PUZZLE_DIFFICULTIES,
  RUN_KINDS,
  VOICE_OUTCOMES,
  VOICE_SOURCES,
  type BestMoveOptions,
  type VoiceAttemptInput,
  type VoiceAttemptUpdate,
  type EngineLevel,
  type LocalLadderQuery,
  type LocalPuzzleQuery,
  type PuzzleRequest,
  type PuzzleSolveRequest,
  type RunInput,
  type RunKind,
  type AppTheme,
  type NotificationRequest,
  type OnlineAction,
  type OnlineOptions,
  type Settings,
} from './types.ts'
import { FEN, GAME_ID, PUZZLE_ANGLE, PUZZLE_ID, UCI_MOVE, USERNAME } from './patterns.ts'

export { FEN, GAME_ID, PUZZLE_ANGLE, PUZZLE_ID, UCI_MOVE, USERNAME }
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
    lightTheme: v.pipe(
      v.string('Invalid color theme.'),
      v.regex(/^[a-z0-9-]{1,32}$/, 'Invalid color theme.'),
    ),
    darkTheme: v.pipe(
      v.string('Invalid color theme.'),
      v.regex(/^[a-z0-9-]{1,32}$/, 'Invalid color theme.'),
    ),
    pieceSet: v.pipe(
      v.string('Invalid piece set.'),
      v.regex(/^[a-z0-9-]{1,32}$/, 'Invalid piece set.'),
    ),
    pieceAnimation: v.picklist(PIECE_ANIMATIONS, 'Invalid piece animation.'),
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
    notificationsEnabled: v.boolean('Invalid notification setting.'),
    notifyActive: v.boolean('Invalid notification setting.'),
    notifyBackground: v.boolean('Invalid notification setting.'),
    notifyOpponentMove: v.boolean('Invalid notification setting.'),
    notifyLowTime: v.boolean('Invalid notification setting.'),
    notifyGameEvents: v.boolean('Invalid notification setting.'),
    notifyComputerMove: v.boolean('Invalid notification setting.'),
    notifySound: v.boolean('Invalid notification setting.'),
    voicePushToTalk: v.boolean('Invalid voice setting.'),
    voiceConfirmMoves: v.boolean('Invalid voice setting.'),
    voiceHistory: v.boolean('Invalid voice setting.'),
    updateAutoCheck: v.boolean('Invalid update setting.'),
    updateAutoDownload: v.boolean('Invalid update setting.'),
    updateInstallOnQuit: v.boolean('Invalid update setting.'),
  },
  'Invalid settings.',
)

const hexColor = v.pipe(
  v.string('Colors must be hex, like #1a2b3c.'),
  v.regex(/^#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/, 'Colors must be hex, like #1a2b3c.'),
)
const paletteSchema = v.object({
  bg: hexColor,
  text: hexColor,
  primary: hexColor,
  muted: v.optional(hexColor),
  elevated: v.optional(hexColor),
  accented: v.optional(hexColor),
  border: v.optional(hexColor),
  textMuted: v.optional(hexColor),
  success: v.optional(hexColor),
  warning: v.optional(hexColor),
  error: v.optional(hexColor),
  info: v.optional(hexColor),
})
const themeSchema = v.pipe(
  v.object(
    {
      id: v.pipe(
        v.string('A theme needs an "id".'),
        v.regex(/^[a-z0-9-]{1,32}$/, 'The "id" must be 1–32 characters: a–z, 0–9 and dashes.'),
      ),
      name: v.pipe(
        v.string('A theme needs a "name".'),
        v.trim(),
        v.minLength(1, 'A theme needs a "name".'),
        v.maxLength(40, 'The "name" is too long.'),
      ),
      light: v.optional(paletteSchema),
      dark: v.optional(paletteSchema),
    },
    'A theme file must be a JSON object.',
  ),
  v.check((theme) => Boolean(theme.light || theme.dark), 'Add a "dark" or "light" palette.'),
)

const notificationSchema = v.object(
  {
    kind: v.picklist(NOTIFICATION_KINDS, 'Invalid notification.'),
    title: v.pipe(v.string('Invalid notification.'), v.maxLength(120, 'Invalid notification.')),
    body: v.pipe(v.string('Invalid notification.'), v.maxLength(400, 'Invalid notification.')),
  },
  'Invalid notification.',
)

/** A connected account, or empty for "no account" (anonymous puzzles). */
const optionalAccountSchema = v.union([v.literal(''), usernameSchema], 'Invalid account.')
const angleSchema = v.pipe(
  v.string('Invalid puzzle theme.'),
  v.regex(PUZZLE_ANGLE, 'Invalid puzzle theme.'),
)
const intInRange = (min: number, max: number, message: string) =>
  v.pipe(v.number(message), v.integer(message), v.minValue(min, message), v.maxValue(max, message))

const puzzleRequestSchema = v.object(
  {
    account: optionalAccountSchema,
    angle: angleSchema,
    difficulty: v.picklist(PUZZLE_DIFFICULTIES, 'Invalid difficulty.'),
    color: v.optional(v.picklist(['white', 'black'] as const, 'Invalid color.')),
  },
  'Invalid puzzle request.',
)
const puzzleSolveSchema = v.object(
  {
    account: usernameSchema,
    angle: angleSchema,
    id: v.pipe(v.string('Invalid puzzle.'), v.regex(PUZZLE_ID, 'Invalid puzzle.')),
    win: v.boolean('Invalid puzzle result.'),
    rated: v.boolean('Invalid puzzle result.'),
  },
  'Invalid puzzle result.',
)
const daysSchema = intInRange(1, 365, 'Invalid number of days.')
const activityMaxSchema = intInRange(1, 200, 'Invalid number of puzzles.')
const localQuerySchema = v.object(
  {
    theme: v.optional(angleSchema),
    minRating: v.optional(intInRange(0, 4000, 'Invalid rating.')),
    maxRating: v.optional(intInRange(0, 4000, 'Invalid rating.')),
    count: intInRange(1, 300, 'Invalid puzzle count.'),
  },
  'Invalid puzzle query.',
)
const ladderQuerySchema = v.object(
  {
    from: intInRange(0, 4000, 'Invalid rating.'),
    to: intInRange(0, 4000, 'Invalid rating.'),
    count: intInRange(1, 300, 'Invalid puzzle count.'),
  },
  'Invalid puzzle query.',
)
const runKindSchema = v.picklist(RUN_KINDS, 'Invalid kind of run.')
const runInputSchema = v.object(
  {
    kind: runKindSchema,
    variant: v.pipe(v.string('Invalid run.'), v.regex(/^[a-zA-Z0-9_-]{0,40}$/, 'Invalid run.')),
    score: v.pipe(
      v.number('Invalid score.'),
      v.finite('Invalid score.'),
      v.minValue(0),
      v.maxValue(1_000_000),
    ),
    detail: v.pipe(
      v.record(
        v.pipe(v.string(), v.regex(/^[a-zA-Z0-9_]{1,30}$/, 'Invalid run.')),
        v.union([v.pipe(v.string(), v.maxLength(80)), v.pipe(v.number(), v.finite()), v.boolean()]),
        'Invalid run.',
      ),
      v.check((detail) => Object.keys(detail).length <= 20, 'Invalid run.'),
    ),
  },
  'Invalid run.',
)
const voiceText = (max: number) => v.pipe(v.string('Invalid voice entry.'), v.maxLength(max))
const voiceOutcomeSchema = v.picklist(VOICE_OUTCOMES, 'Invalid voice entry.')
const voiceIdSchema = intInRange(1, Number.MAX_SAFE_INTEGER, 'Invalid voice entry.')
const voiceAttemptSchema = v.object(
  {
    source: v.picklist(VOICE_SOURCES, 'Invalid voice entry.'),
    heard: voiceText(200),
    confidence: v.pipe(v.number('Invalid voice entry.'), v.minValue(0), v.maxValue(1)),
    words: v.pipe(
      v.array(
        v.object({
          word: voiceText(40),
          conf: v.pipe(v.number('Invalid voice entry.'), v.minValue(0), v.maxValue(1)),
        }),
      ),
      v.maxLength(40, 'Invalid voice entry.'),
    ),
    outcome: voiceOutcomeSchema,
    parsed: v.optional(voiceText(120)),
    expected: v.optional(voiceText(20)),
    fen: v.optional(v.pipe(voiceText(100), v.regex(FEN, 'Invalid position.'))),
    retryOf: v.optional(voiceIdSchema),
  },
  'Invalid voice entry.',
)
const voiceUpdateSchema = v.object(
  { outcome: v.optional(voiceOutcomeSchema), expected: v.optional(voiceText(20)) },
  'Invalid voice entry.',
)
const bestMoveOptionsSchema = v.optional(
  v.object(
    {
      fen: v.optional(
        v.pipe(v.string('Invalid position.'), v.maxLength(100), v.regex(FEN, 'Invalid position.')),
      ),
      movetime: v.optional(intInRange(50, 5000, 'Invalid think time.')),
    },
    'Invalid engine options.',
  ),
  {},
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
export const assertTheme = (value: unknown): AppTheme => parse(themeSchema, value)
export const assertNotification = (value: unknown): NotificationRequest =>
  parse(notificationSchema, value)
export const assertSettings = (value: unknown): Settings => parse(settingsSchema, value)
export const assertPuzzleRequest = (value: unknown): PuzzleRequest =>
  parse(puzzleRequestSchema, value)
export const assertPuzzleSolve = (value: unknown): PuzzleSolveRequest =>
  parse(puzzleSolveSchema, value)
export const assertDays = (value: unknown): number => parse(daysSchema, value)
export const assertActivityMax = (value: unknown): number => parse(activityMaxSchema, value)
export const assertLocalQuery = (value: unknown): LocalPuzzleQuery => parse(localQuerySchema, value)
export const assertLadderQuery = (value: unknown): LocalLadderQuery =>
  parse(ladderQuerySchema, value)
export const assertRunKind = (value: unknown): RunKind => parse(runKindSchema, value)
export const assertRunInput = (value: unknown): RunInput => parse(runInputSchema, value)
export const assertBestMoveOptions = (value: unknown): BestMoveOptions =>
  parse(bestMoveOptionsSchema, value)
export const assertVoiceAttempt = (value: unknown): VoiceAttemptInput =>
  parse(voiceAttemptSchema, value)
export const assertVoiceUpdate = (value: unknown): VoiceAttemptUpdate =>
  parse(voiceUpdateSchema, value)
export const assertVoiceId = (value: unknown): number => parse(voiceIdSchema, value)
export const assertVoiceLimit = (value: unknown): number =>
  parse(intInRange(1, 5000, 'Invalid number of entries.'), value)
