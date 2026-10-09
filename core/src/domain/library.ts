import type { EngineLevel, TournamentSystem } from '../contracts/types'
import type { GameSetup } from './variant'
import { rules, rulesLossless } from './engine.ts'

/**
 * The local library: bounded, versioned documents the core keeps for every frontend
 * (studies, played games, mistake drills, unfinished sessions and training notes). The decoders
 * and checks are the Rust validators in `crates/kchess-domain/src/misc/documents.rs`.
 */

type Color = 'white' | 'black'

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

export function decodeAnalysisSession(raw: unknown): AnalysisSession | undefined {
  return rulesLossless<AnalysisSession | null>('decodeAnalysisSession', raw) ?? undefined
}

export function decodeLocalSession(raw: unknown): LocalSession | undefined {
  return rulesLossless<LocalSession | null>('decodeLocalSession', raw) ?? undefined
}

export function decodeComputerSession(raw: unknown): ComputerSession | undefined {
  return rulesLossless<ComputerSession | null>('decodeComputerSession', raw) ?? undefined
}

export function decodeArchiveIdentity(raw: unknown): ArchiveIdentity | undefined {
  return rulesLossless<ArchiveIdentity | null>('decodeArchiveIdentity', raw) ?? undefined
}

/** A stored (versioned) session document. */
export function decodeStoredSession<K extends SessionKind>(
  kind: K,
  raw: unknown,
): SessionDocuments[K] | undefined {
  return rulesLossless<SessionDocuments[K] | null>('decodeStoredSession', kind, raw) ?? undefined
}

/** Add the on-disk version to a decoded session. */
export function encodeSession<K extends SessionKind>(
  kind: K,
  session: SessionDocuments[K],
): object {
  return rulesLossless<object>('encodeSession', kind, session)
}

export const isSessionKind = (value: unknown): value is SessionKind =>
  rulesLossless<boolean>('isSessionKind', value)

/** A session a frontend sends; throws when it is not valid for its kind. */
export function assertSession<K extends SessionKind>(kind: K, value: unknown): SessionDocuments[K] {
  return rulesLossless<SessionDocuments[K]>('assertSession', kind, value)
}

export function decodeJoinedTournaments(raw: unknown): JoinedTournament[] {
  return rulesLossless<JoinedTournament[]>('decodeJoinedTournaments', raw)
}

export function decodeRepertoireMisses(raw: unknown): RepertoireMisses {
  return rulesLossless<RepertoireMisses>('decodeRepertoireMisses', raw)
}

/* ── Studies: whole-document PGN, as exported and compared with the cloud copy ── */

/** Every chapter as one PGN, each named by a `ChapterName` header. */
export function studyDocumentPgn(study: Pick<SavedStudy, 'chapters'>): string {
  const pgn = rules<string | null>(
    'studyDocumentPgn',
    study.chapters.map((c) => ({ name: c.name, pgn: c.pgn })),
  )
  // Saved chapters are valid PGN documents, so each holds a game.
  if (pgn === null) throw new Error('A study chapter holds no game.')
  return pgn
}

/* ── Study commands a frontend sends to the core ── */

export type StudyCommand =
  Exclude<StudyCommandInput, { op: 'restore' }> | { op: 'restore'; study: SavedStudy }

export interface StudyCommandResult {
  studies: SavedStudy[]
  /** The study or chapter the command created or selected. */
  id?: string
  /** `offline`: the device copy had local edits, so the cloud version became a new copy. */
  conflict?: boolean
  /** `remove`: what was removed, for undo. */
  removed?: SavedStudy
}

/**
 * A study command's shape, as a frontend sends it. A study to restore is checked by the core
 * service, whose rules replay every chapter (`assertStudyCommand` in `services/rules.ts`).
 */
export type StudyCommandInput =
  | { op: 'save'; name: string; pgn: string; id?: string; chapterId?: string }
  | { op: 'saveChapters'; name: string; chapters: { name: string; pgn: string }[]; id?: string }
  | { op: 'addChapter'; id: string; name: string }
  | { op: 'renameChapter'; id: string; chapterId: string; name: string }
  | { op: 'duplicateChapter'; id: string; chapterId: string }
  | { op: 'removeChapter'; id: string; chapterId: string }
  | {
      op: 'markCloud'
      id: string
      account: string
      remoteId: string
      baseline: string
    }
  | {
      op: 'offline'
      account: string
      remote: { id: string; name: string }
      chapters: { name: string; pgn: string }[]
    }
  | { op: 'remove'; id: string }
  | { op: 'restore'; study: unknown }
  | { op: 'rename'; id: string; name: string }
  | { op: 'duplicate'; id: string }
export function assertStudyCommandShape(value: unknown): StudyCommandInput {
  return rulesLossless<StudyCommandInput>('assertStudyCommandShape', value)
}

/** An archived game is an object; the core service checks its fields and replays its moves. */
export function assertArchivedGameShape(value: unknown): object {
  rulesLossless<unknown>('assertArchivedGameShape', value)
  return value as object
}

export const assertLibraryId = (value: unknown): string =>
  rulesLossless<string>('assertLibraryId', value)
export const assertRepertoireKey = (value: unknown): string =>
  rulesLossless<string>('assertRepertoireKey', value)
export const assertSessionKind = (value: unknown): SessionKind =>
  rulesLossless<SessionKind>('assertSessionKind', value)
export const assertSide = (value: unknown): Color => rulesLossless<Color>('assertSide', value)
export const assertLegacyDocuments = (value: unknown): LegacyDocuments =>
  rulesLossless<LegacyDocuments>('assertLegacyDocuments', value)
