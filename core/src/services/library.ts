import type {
  ArchivedGame,
  JoinedTournament,
  LegacyDocuments,
  LibrarySnapshot,
  MistakeExercise,
  RepertoireMisses,
  SessionDocuments,
  SessionKind,
  StudyCommand,
  StudyCommandResult,
} from '../domain/library'
import type { TournamentSystem } from '../contracts/types'
import { nativeCallSync } from './nativeCore'

/**
 * The core's library (studies, played games, mistake drills, unfinished sessions, joined
 * tournaments and repertoire notes): bounded, versioned documents in the Rust core's `kchess.db`
 * (`crates/kchess-core/src/store/library.rs`). This module keeps the calls only.
 */

export function readSession<K extends SessionKind>(kind: K): SessionDocuments[K] | undefined {
  return (nativeCallSync<SessionDocuments[K] | null>('store.library.readSession', kind) ??
    undefined) as SessionDocuments[K] | undefined
}

export function library(now = Date.now()): LibrarySnapshot {
  return nativeCallSync<LibrarySnapshot>('store.library.library', now)
}

/**
 * Take over documents an earlier release kept in the renderer, once. Each is validated as the
 * library would; invalid ones are skipped. Nothing already in the library is replaced.
 */
export function importLibrary(documents: LegacyDocuments): LibrarySnapshot {
  return nativeCallSync<LibrarySnapshot>('store.library.importLibrary', documents)
}

/** Apply one change to the study library and store it; the library is unchanged on failure. */
export function studyCommand(command: StudyCommand, now = Date.now()): StudyCommandResult {
  return nativeCallSync<StudyCommandResult>('store.library.studyCommand', command, now)
}

/** Add or replace a game; it becomes the most recent. */
export function saveArchivedGame(game: ArchivedGame): void {
  nativeCallSync('store.library.saveArchivedGame', game)
}

export function removeArchivedGame(id: string): void {
  nativeCallSync('store.library.removeArchivedGame', id)
}

/** Turn a completed review's mistakes (optionally one side's) into drills; returns how many were new. */
export function addMistakes(
  reviewKey: string,
  color?: 'white' | 'black',
  now = Date.now(),
): { added: number; items: MistakeExercise[] } {
  return nativeCallSync<{ added: number; items: MistakeExercise[] }>(
    'store.library.addMistakes',
    reviewKey,
    color ?? null,
    now,
  )
}

/** Schedule a drill again: soon after a miss, further out after each clean solve. */
export function answerMistake(id: string, solved: boolean, now = Date.now()): MistakeExercise[] {
  return nativeCallSync<MistakeExercise[]>('store.library.answerMistake', id, solved, now)
}

export function saveSession<K extends SessionKind>(kind: K, value: SessionDocuments[K]): void {
  nativeCallSync('store.library.saveSession', kind, value)
}

export function joinedTournaments(now = Date.now()): JoinedTournament[] {
  return nativeCallSync<JoinedTournament[]>('store.library.joinedTournaments', now)
}

export function rememberTournament(entry: JoinedTournament, now = Date.now()): void {
  nativeCallSync('store.library.rememberTournament', entry, now)
}

export function forgetTournament(system: TournamentSystem, id: string): void {
  nativeCallSync('store.library.forgetTournament', system, id)
}

/** Signed-out accounts must not reopen event streams for their tournaments. */
export function forgetTournamentsOf(accounts: readonly string[]): void {
  nativeCallSync('store.library.forgetTournamentsOf', accounts)
}

export function recordRepertoireMiss(key: string, fen: string): RepertoireMisses {
  return nativeCallSync<RepertoireMisses>('store.library.recordRepertoireMiss', key, fen)
}

export function clearRepertoireMisses(key: string): RepertoireMisses {
  return nativeCallSync<RepertoireMisses>('store.library.clearRepertoireMisses', key)
}
