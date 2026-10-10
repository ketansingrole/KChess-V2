import { rules, rulesLossless } from './engine.ts'
import type {
  Color,
  SavedStudy,
  JoinedTournament,
  RepertoireMisses,
  AnalysisSession,
  LocalSession,
  ComputerSession,
  ArchiveIdentity,
  SessionDocuments,
  SessionKind,
  LegacyDocuments,
  StudyCommandInput,
} from '../contracts/generated/library.ts'
export { SESSION_KINDS, LEGACY_DOCUMENT_KEYS } from '../contracts/generated/library.ts'
export type {
  Color,
  StudyChapter,
  SavedStudy,
  ArchivedGame,
  GameSnapshot,
  MistakeExercise,
  JoinedTournament,
  RepertoireMisses,
  LocalClock,
  AnalysisSession,
  LocalSession,
  ComputerSession,
  ArchiveIdentity,
  SessionDocuments,
  SessionKind,
  LibrarySnapshot,
  LegacyDocuments,
  StudyCommand,
  StudyCommandResult,
  StudyCommandInput,
} from '../contracts/generated/library.ts'

/**
 * The local library: bounded, versioned documents the core keeps for every frontend
 * (studies, played games, mistake drills, unfinished sessions and training notes). The decoders
 * and checks are the Rust validators in `crates/kchess-domain/src/misc/documents.rs`.
 */

/** Largest serialized document the core stores. */
export const MAX_DOCUMENT = 2_000_000
export const MAX_STUDIES = 50
export const MAX_CHAPTERS = 64
export const MAX_ARCHIVED_GAMES = 500
export const MAX_MISTAKES = 500

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
