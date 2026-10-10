/**
 * Lichess studies: listing them, reading their chapters, exporting a study and syncing it. The Rust
 * core does the work (`lichess/studies.rs`).
 */
import type {
  LichessStudy,
  LichessStudyChapter,
  NeedsReconnect,
  StudySyncRequest,
} from '../contracts/types'
import { nativeCall, nativeCallSync } from './nativeCore'

/** The studies an account owns or belongs to (private ones too, with `study:read`). */
export function lichessStudies(account: string): Promise<LichessStudy[] | NeedsReconnect> {
  return nativeCall('studies.list', account)
}

/** Split a multi-game PGN into games (Lichess separates them with blank lines before a tag). */
export function splitPgn(text: string): string[] {
  return nativeCallSync('studies.splitPgn', text)
}

export function lichessStudyChapters(
  account: string,
  id: string,
): Promise<LichessStudyChapter[] | NeedsReconnect> {
  return nativeCall('studies.chapters', account, id)
}

/** Exports a game or study to a Lichess study; `studyId` `''` creates a new study named `name`. */
export function exportToLichessStudy(
  account: string,
  studyId: string,
  name: string,
  pgn: string,
): Promise<{ id: string } | NeedsReconnect> {
  return nativeCall('studies.export', account, studyId, name, pgn)
}

export function syncLichessStudy(
  request: StudySyncRequest,
): Promise<LichessStudyChapter[] | NeedsReconnect> {
  return nativeCall('studies.sync', request)
}
