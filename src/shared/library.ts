import * as v from 'valibot'
import { makePgn, parsePgn } from 'chessops/pgn'
import { nodesAlong, pathOf, treeFromPgn } from './analysisTree'
import { UCI_MOVE } from './patterns'
import { replay } from './review'
import { ENGINE_LEVELS, type EngineLevel, type TournamentSystem } from './types'
import { engineSupports, isVariant, replaySetup, STANDARD_SETUP, type GameSetup } from './variant'

/**
 * The local library: bounded, versioned documents the core keeps for every frontend
 * (studies, played games, mistake drills, unfinished sessions and training notes).
 */

type Color = 'white' | 'black'
const TOURNAMENT_SYSTEMS: readonly TournamentSystem[] = ['arena', 'swiss']

/** Largest serialized document the core stores. */
export const MAX_DOCUMENT = 2_000_000
export const MAX_STUDIES = 50
export const MAX_CHAPTERS = 64
export const MAX_ARCHIVED_GAMES = 500
export const MAX_MISTAKES = 500

export interface StudyChapter {
  id: string
  name: string
  pgn: string
}
export interface SavedStudy {
  id: string
  name: string
  /** First chapter, retained for existing repertoire and preview consumers. */
  pgn: string
  chapters: StudyChapter[]
  updatedAt: number
  cloud?: { account: string; id: string; downloadedPgn: string; structureChanged?: boolean }
}

export interface ArchivedGame {
  id: string
  source: 'computer' | 'board' | 'clock'
  startedAt: number
  updatedAt: number
  white: string
  black: string
  result: '*' | '1-0' | '0-1' | '1/2-1/2'
  reason: string
  finished: boolean
  setup: GameSetup
  moves: string[]
  timeControl: string
  clockSummary?: string
}
export type GameSnapshot = Omit<ArchivedGame, 'id' | 'startedAt' | 'updatedAt' | 'finished'>

export interface MistakeExercise {
  id: string
  fen: string
  solution: string[]
  judgment: string
  dueAt: number
  streak: number
  attempts: number
}

/** A tournament joined from KChess; its pairings arrive on the account's event stream. */
export interface JoinedTournament {
  system: TournamentSystem
  id: string
  account: string
  name: string
  /** Stop keeping the event stream open for it after this time. */
  until: number
}

/** Missed repertoire moves by `studyId:color`, then by FEN. */
export type RepertoireMisses = Record<string, Record<string, number>>

/** Each side's clock: minutes and seconds added after every move (sides may differ, for odds). */
export interface LocalClock {
  white: { minutes: number; increment: number }
  black: { minutes: number; increment: number }
}

export interface AnalysisSession {
  pgn: string
  path: string
  orientation: Color
  study: string
  chapter: string
}
export interface LocalSession {
  setup: GameSetup
  moves: string[]
  clock: LocalClock | null
  times: { white: number; black: number } | null
  result: { winner?: Color; reason: string } | null
}
export interface ComputerSession {
  moves: string[]
  ply: number
  level: EngineLevel
  color: Color
  resigned: boolean
  setup: GameSetup
  clock: { minutes: number; increment: number } | null
  times: { white: number; black: number } | null
  flagged: Color | null
}
/** Which archive entry an unfinished game updates, across reloads. */
export interface ArchiveIdentity {
  id: string
  startedAt: number
}

export interface SessionDocuments {
  analysis: AnalysisSession
  local: LocalSession
  computer: ComputerSession
  'archive:computer': ArchiveIdentity
  'archive:board': ArchiveIdentity
  'archive:clock': ArchiveIdentity
}
export type SessionKind = keyof SessionDocuments
export const SESSION_KINDS = [
  'analysis',
  'local',
  'computer',
  'archive:computer',
  'archive:board',
  'archive:clock',
] as const satisfies readonly SessionKind[]

export interface LibrarySnapshot {
  studies: SavedStudy[]
  games: ArchivedGame[]
  mistakes: MistakeExercise[]
  sessions: Partial<SessionDocuments>
  joinedTournaments: JoinedTournament[]
  repertoireMisses: RepertoireMisses
  /** False until documents saved by an earlier release have been offered for import. */
  imported: boolean
}

/** Where earlier releases kept each document in the renderer's local storage. */
export const LEGACY_DOCUMENT_KEYS = [
  'kchess:studies:v1',
  'kchess:game-history:v1',
  'kchess:mistakes:v1',
  'kchess:analysis:v1',
  'kchess:local:v1',
  'kchess:computer:v1',
  'kchess:history-session:computer',
  'kchess:history-session:board',
  'kchess:history-session:clock',
  'kchess:tournaments-joined',
  'kchess:repertoire-misses',
] as const
export type LegacyDocuments = Partial<Record<(typeof LEGACY_DOCUMENT_KEYS)[number], string>>

/* ── Decoders: semantic validation before a document reaches the library or a board ── */

const text = (value: unknown, max: number): value is string =>
  typeof value === 'string' && value.length <= max

function decodeChapters(raw: unknown, fallback: StudyChapter): StudyChapter[] | undefined {
  const chapters = raw ?? [fallback]
  if (
    !Array.isArray(chapters) ||
    !chapters.length ||
    chapters.length > MAX_CHAPTERS ||
    chapters.some(
      (c) =>
        !c ||
        typeof c.id !== 'string' ||
        typeof c.name !== 'string' ||
        typeof c.pgn !== 'string' ||
        !treeFromPgn(c.pgn),
    )
  )
    return undefined
  return chapters.map((c: StudyChapter) => ({ id: c.id, name: c.name, pgn: c.pgn }))
}

/** One saved study, or undefined when any chapter is not a valid PGN document. */
export function decodeStudy(raw: unknown): SavedStudy | undefined {
  if (!raw || typeof raw !== 'object') return undefined
  const item = raw as Record<string, unknown> & Partial<SavedStudy>
  if (
    typeof item.id !== 'string' ||
    typeof item.name !== 'string' ||
    typeof item.pgn !== 'string' ||
    !Number.isFinite(item.updatedAt)
  )
    return undefined
  const chapters = decodeChapters(item.chapters, { id: item.id, name: item.name, pgn: item.pgn })
  if (!chapters) return undefined
  const cloud =
    item.cloud &&
    typeof item.cloud.account === 'string' &&
    typeof item.cloud.id === 'string' &&
    /^[a-zA-Z0-9]{8}$/.test(item.cloud.id) &&
    typeof item.cloud.downloadedPgn === 'string'
      ? {
          account: item.cloud.account,
          id: item.cloud.id,
          downloadedPgn: item.cloud.downloadedPgn,
          ...(item.cloud.structureChanged === true ? { structureChanged: true } : {}),
        }
      : undefined
  return {
    id: item.id,
    name: item.name,
    pgn: chapters[0]!.pgn,
    chapters,
    updatedAt: item.updatedAt as number,
    ...(cloud ? { cloud } : {}),
  }
}

/** The study library; invalid studies are left out rather than failing the whole library. */
export function decodeStudies(raw: unknown): SavedStudy[] | undefined {
  if (
    !raw ||
    typeof raw !== 'object' ||
    !('version' in raw) ||
    ![1, 2].includes(Number(raw.version)) ||
    !('items' in raw) ||
    !Array.isArray(raw.items)
  )
    return undefined
  return raw.items.slice(0, MAX_STUDIES).flatMap((item) => decodeStudy(item) ?? [])
}

export function decodeArchivedGame(raw: unknown): ArchivedGame | undefined {
  if (!raw || typeof raw !== 'object') return undefined
  const g = raw as ArchivedGame
  if (
    typeof g.id !== 'string' ||
    g.id.length > 80 ||
    !['computer', 'board', 'clock'].includes(g.source) ||
    !Number.isFinite(g.startedAt) ||
    !Number.isFinite(g.updatedAt) ||
    !['*', '1-0', '0-1', '1/2-1/2'].includes(g.result) ||
    typeof g.finished !== 'boolean' ||
    ![g.white, g.black, g.reason, g.timeControl].every((s) => text(s, 200)) ||
    (g.clockSummary !== undefined && !text(g.clockSummary, 200)) ||
    !g.setup ||
    !isVariant(g.setup.variant) ||
    !text(g.setup.fen, 120) ||
    !Array.isArray(g.moves) ||
    g.moves.length > 1024 ||
    !g.moves.every((m) => text(m, 10)) ||
    replaySetup(g.setup, g.moves)?.played.length !== g.moves.length
  )
    return undefined
  return {
    id: g.id,
    source: g.source,
    startedAt: g.startedAt,
    updatedAt: g.updatedAt,
    white: g.white,
    black: g.black,
    result: g.result,
    reason: g.reason,
    finished: g.finished,
    setup: { variant: g.setup.variant, fen: g.setup.fen },
    moves: [...g.moves],
    timeControl: g.timeControl,
    ...(g.clockSummary !== undefined ? { clockSummary: g.clockSummary } : {}),
  }
}

/** Played games; any invalid or duplicated game rejects the document. */
export function decodeArchive(raw: unknown): ArchivedGame[] | undefined {
  if (!raw || typeof raw !== 'object') return
  const doc = raw as { version?: unknown; games?: unknown }
  if (doc.version !== 1 || !Array.isArray(doc.games) || doc.games.length > MAX_ARCHIVED_GAMES)
    return
  const games: ArchivedGame[] = []
  const ids = new Set<string>()
  for (const rawGame of doc.games) {
    const game = decodeArchivedGame(rawGame)
    if (!game || ids.has(game.id)) return
    ids.add(game.id)
    games.push(game)
  }
  return games
}

export function decodeMistakes(raw: unknown): MistakeExercise[] | undefined {
  if (
    !raw ||
    typeof raw !== 'object' ||
    !('version' in raw) ||
    raw.version !== 1 ||
    !('items' in raw) ||
    !Array.isArray(raw.items)
  )
    return undefined
  return raw.items.slice(0, MAX_MISTAKES).flatMap((item: MistakeExercise) =>
    item &&
    typeof item.id === 'string' &&
    typeof item.fen === 'string' &&
    Array.isArray(item.solution) &&
    item.solution.every((move: unknown) => typeof move === 'string') &&
    replay(item.fen, item.solution).length === item.solution.length + 1 &&
    Number.isFinite(item.dueAt) &&
    Number.isInteger(item.streak) &&
    Number.isInteger(item.attempts)
      ? [
          {
            id: item.id,
            fen: item.fen,
            solution: [...item.solution],
            judgment: String(item.judgment),
            dueAt: item.dueAt,
            streak: item.streak,
            attempts: item.attempts,
          },
        ]
      : [],
  )
}

/**
 * The last document parsed: sessions are saved after every move along the board as well as after
 * edits, and moving changes only the path.
 */
let parsed: { pgn: string; root: ReturnType<typeof treeFromPgn> } | undefined

export function decodeAnalysisSession(raw: unknown): AnalysisSession | undefined {
  if (!raw || typeof raw !== 'object') return undefined
  const value = raw as Record<string, unknown>
  if (value.version !== 1 || typeof value.pgn !== 'string') return undefined
  if (parsed?.pgn !== value.pgn) parsed = { pgn: value.pgn, root: treeFromPgn(value.pgn) }
  const root = parsed.root
  if (!root) return undefined
  // A path the document no longer contains ends where the document does.
  const path =
    typeof value.path === 'string'
      ? pathOf(
          nodesAlong(root, value.path)
            .slice(1)
            .map((n) => n.uci),
        )
      : ''
  return {
    pgn: value.pgn,
    path,
    orientation: value.orientation === 'black' ? 'black' : 'white',
    study: typeof value.study === 'string' ? value.study : '',
    chapter: typeof value.chapter === 'string' ? value.chapter : '',
  }
}

const ms = (n: unknown): number =>
  typeof n === 'number' && Number.isFinite(n) ? Math.max(0, n) : 0
const clockSide = (raw: unknown): { minutes: number; increment: number } | undefined => {
  if (!raw || typeof raw !== 'object') return undefined
  const { minutes, increment } = raw as Record<string, unknown>
  return Number.isInteger(minutes) &&
    Number.isInteger(increment) &&
    (minutes as number) >= 0 &&
    (minutes as number) <= 180 &&
    (increment as number) >= 0 &&
    (increment as number) <= 180
    ? { minutes: minutes as number, increment: increment as number }
    : undefined
}
const playable = (setup: GameSetup, moves: unknown): moves is string[] =>
  Array.isArray(moves) &&
  moves.length <= 1024 &&
  moves.every((m) => typeof m === 'string' && UCI_MOVE.test(m)) &&
  replaySetup(setup, moves)?.played.length === moves.length

export function decodeLocalSession(raw: unknown): LocalSession | undefined {
  const value = raw as {
    version?: unknown
    setup?: { variant?: unknown; fen?: unknown }
    moves?: unknown
    clock?: { white?: unknown; black?: unknown } | null
    times?: { white?: unknown; black?: unknown } | null
    result?: { winner?: unknown; reason?: unknown } | null
  }
  if (!value || value.version !== 1 || !value.setup || !isVariant(value.setup.variant)) return
  if (!text(value.setup.fen, 120)) return
  const setup = { variant: value.setup.variant, fen: value.setup.fen }
  if (!playable(setup, value.moves)) return
  const white = clockSide(value.clock?.white)
  const black = clockSide(value.clock?.black)
  return {
    setup,
    moves: [...value.moves],
    clock: white && black ? { white, black } : null,
    times: value.times ? { white: ms(value.times.white), black: ms(value.times.black) } : null,
    result:
      value.result && typeof value.result.reason === 'string'
        ? {
            ...(value.result.winner === 'white' || value.result.winner === 'black'
              ? { winner: value.result.winner }
              : {}),
            reason: value.result.reason.slice(0, 80),
          }
        : null,
  }
}

export function decodeComputerSession(raw: unknown): ComputerSession | undefined {
  if (!raw || typeof raw !== 'object') return undefined
  const value = raw as {
    version?: unknown
    moves?: unknown
    ply?: unknown
    level?: unknown
    color?: unknown
    resigned?: unknown
    setup?: { variant?: unknown; fen?: unknown }
    clock?: unknown
    times?: { white?: unknown; black?: unknown }
    flagged?: unknown
  }
  // Version 1 games began at the standard start and had no clock.
  const setup: GameSetup =
    value.version === 2 &&
    value.setup &&
    isVariant(value.setup.variant) &&
    text(value.setup.fen, 100)
      ? { variant: value.setup.variant, fen: value.setup.fen }
      : STANDARD_SETUP
  if (
    (value.version !== 1 && value.version !== 2) ||
    !engineSupports(setup.variant) ||
    !playable(setup, value.moves) ||
    !ENGINE_LEVELS.includes(value.level as EngineLevel) ||
    !['white', 'black'].includes(String(value.color))
  )
    return undefined
  const clock = clockSide(value.clock) ?? null
  return {
    moves: [...value.moves],
    ply:
      typeof value.ply === 'number'
        ? Math.max(0, Math.min(value.moves.length, Math.floor(value.ply)))
        : value.moves.length,
    level: value.level as EngineLevel,
    color: value.color as Color,
    resigned: value.resigned === true,
    setup,
    clock,
    times: clock ? { white: ms(value.times?.white), black: ms(value.times?.black) } : null,
    flagged: value.flagged === 'white' || value.flagged === 'black' ? value.flagged : null,
  }
}

export function decodeArchiveIdentity(raw: unknown): ArchiveIdentity | undefined {
  if (!raw || typeof raw !== 'object') return undefined
  const { id, startedAt } = raw as Record<string, unknown>
  return text(id, 80) && id && Number.isFinite(startedAt)
    ? { id, startedAt: startedAt as number }
    : undefined
}

/** Session documents carry their version on disk; frontends exchange the decoded shape. */
const SESSION_VERSION: Record<SessionKind, number | undefined> = {
  analysis: 1,
  local: 1,
  computer: 2,
  'archive:computer': undefined,
  'archive:board': undefined,
  'archive:clock': undefined,
}
const SESSION_DECODERS: { [K in SessionKind]: (raw: unknown) => SessionDocuments[K] | undefined } =
  {
    analysis: decodeAnalysisSession,
    local: decodeLocalSession,
    computer: decodeComputerSession,
    'archive:computer': decodeArchiveIdentity,
    'archive:board': decodeArchiveIdentity,
    'archive:clock': decodeArchiveIdentity,
  }
/** A stored (versioned) session document. */
export function decodeStoredSession<K extends SessionKind>(
  kind: K,
  raw: unknown,
): SessionDocuments[K] | undefined {
  return SESSION_DECODERS[kind](raw) as SessionDocuments[K] | undefined
}
/** Add the on-disk version to a decoded session. */
export function encodeSession<K extends SessionKind>(
  kind: K,
  session: SessionDocuments[K],
): object {
  const version = SESSION_VERSION[kind]
  return version === undefined ? session : { version, ...session }
}
export const isSessionKind = (value: unknown): value is SessionKind =>
  SESSION_KINDS.includes(value as SessionKind)
/** A session a frontend sends; throws when it is not valid for its kind. */
export function assertSession<K extends SessionKind>(kind: K, value: unknown): SessionDocuments[K] {
  const decoded =
    value && typeof value === 'object'
      ? decodeStoredSession(kind, encodeSession(kind, value as SessionDocuments[K]))
      : undefined
  if (!decoded) throw new Error('This session cannot be saved.')
  return decoded
}

export function decodeJoinedTournaments(raw: unknown): JoinedTournament[] {
  if (!Array.isArray(raw)) return []
  return raw.slice(0, 100).flatMap((entry: Partial<JoinedTournament>) =>
    entry &&
    TOURNAMENT_SYSTEMS.includes(entry.system as TournamentSystem) &&
    text(entry.id, 20) &&
    text(entry.account, 40) &&
    text(entry.name, 200) &&
    Number.isFinite(entry.until)
      ? [
          {
            system: entry.system as TournamentSystem,
            id: entry.id as string,
            account: entry.account as string,
            name: entry.name as string,
            until: entry.until as number,
          },
        ]
      : [],
  )
}

export function decodeRepertoireMisses(raw: unknown): RepertoireMisses {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {}
  const result: RepertoireMisses = {}
  for (const [key, table] of Object.entries(raw).slice(0, 200)) {
    if (!text(key, 200) || !table || typeof table !== 'object') continue
    const counts: Record<string, number> = {}
    for (const [fen, count] of Object.entries(table).slice(0, 2000))
      if (text(fen, 100) && Number.isInteger(count) && (count as number) > 0)
        counts[fen] = count as number
    result[key] = counts
  }
  return result
}

/* ── Studies: whole-document PGN, as exported and compared with the cloud copy ── */

/** Every chapter as one PGN, each named by a `ChapterName` header. */
export function studyDocumentPgn(study: Pick<SavedStudy, 'chapters'>): string {
  return study.chapters
    .map((c) => {
      const game = parsePgn(c.pgn)[0]!
      game.headers.set('ChapterName', c.name)
      return makePgn(game)
    })
    .join('\n\n')
}

/** The moves and annotations of a study, ignoring headers Lichess adds or rewrites. */
export function studyContent(pgn: string): string {
  return parsePgn(pgn)
    .map((game) => {
      game.headers.delete('Site')
      game.headers.delete('ChapterName')
      return makePgn(game)
    })
    .join('\n\n')
}

/* ── Study commands a frontend sends to the core ── */

const studyName = v.pipe(v.string(), v.maxLength(200))
const localId = v.pipe(v.string(), v.minLength(1), v.maxLength(80))
const studyPgn = v.pipe(v.string(), v.maxLength(MAX_DOCUMENT))
const lichessId = v.pipe(v.string(), v.regex(/^[a-zA-Z0-9]{8}$/))
const account = v.pipe(v.string(), v.maxLength(40))
const chapterInput = v.pipe(
  v.array(v.object({ name: studyName, pgn: studyPgn })),
  v.minLength(1),
  v.maxLength(MAX_CHAPTERS),
)
const studyCommandSchema = v.variant('op', [
  v.object({
    op: v.literal('save'),
    name: studyName,
    pgn: studyPgn,
    id: v.optional(localId),
    chapterId: v.optional(localId),
  }),
  v.object({
    op: v.literal('saveChapters'),
    name: studyName,
    chapters: chapterInput,
    id: v.optional(localId),
  }),
  v.object({ op: v.literal('addChapter'), id: localId, name: studyName }),
  v.object({ op: v.literal('renameChapter'), id: localId, chapterId: localId, name: studyName }),
  v.object({ op: v.literal('duplicateChapter'), id: localId, chapterId: localId }),
  v.object({ op: v.literal('removeChapter'), id: localId, chapterId: localId }),
  v.object({
    op: v.literal('markCloud'),
    id: localId,
    account,
    remoteId: lichessId,
    baseline: studyPgn,
  }),
  v.object({
    op: v.literal('offline'),
    account,
    remote: v.object({ id: lichessId, name: studyName }),
    chapters: chapterInput,
  }),
  v.object({ op: v.literal('remove'), id: localId }),
  v.object({ op: v.literal('restore'), study: v.unknown() }),
  v.object({ op: v.literal('rename'), id: localId, name: studyName }),
  v.object({ op: v.literal('duplicate'), id: localId }),
])
export type StudyCommand =
  | Exclude<v.InferOutput<typeof studyCommandSchema>, { op: 'restore' }>
  | { op: 'restore'; study: SavedStudy }

export interface StudyCommandResult {
  studies: SavedStudy[]
  /** The study or chapter the command created or selected. */
  id?: string
  /** `offline`: the device copy had local edits, so the cloud version became a new copy. */
  conflict?: boolean
  /** `remove`: what was removed, for undo. */
  removed?: SavedStudy
}

export function assertStudyCommand(value: unknown): StudyCommand {
  const command = v.parse(studyCommandSchema, value)
  if (command.op !== 'restore') return command
  const study = decodeStudy(command.study)
  if (!study) throw new Error('That study cannot be restored.')
  return { op: 'restore', study }
}

export function assertArchivedGame(value: unknown): ArchivedGame {
  const game = decodeArchivedGame(value)
  if (!game) throw new Error('This game cannot be saved.')
  return game
}

const shortText = (max: number) => v.pipe(v.string(), v.minLength(1), v.maxLength(max))
export const assertLibraryId = (value: unknown): string => v.parse(shortText(80), value)
export const assertRepertoireKey = (value: unknown): string => v.parse(shortText(200), value)
export const assertSessionKind = (value: unknown): SessionKind =>
  v.parse(v.picklist(SESSION_KINDS), value)
export const assertSide = (value: unknown): Color => v.parse(v.picklist(['white', 'black']), value)
const legacyDocumentsSchema = v.strictObject(
  Object.fromEntries(
    LEGACY_DOCUMENT_KEYS.map((key) => [
      key,
      v.optional(v.pipe(v.string(), v.maxLength(MAX_DOCUMENT))),
    ]),
  ),
)
export const assertLegacyDocuments = (value: unknown): LegacyDocuments =>
  v.parse(legacyDocumentsSchema, value) as LegacyDocuments
