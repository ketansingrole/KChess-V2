import { randomUUID } from 'node:crypto'
import { analyseReview, replay } from '../shared/review'
import { treeFromPgn } from '../shared/analysisTree'
import {
  LEGACY_DOCUMENT_KEYS,
  MAX_ARCHIVED_GAMES,
  decodeArchivedGame,
  MAX_CHAPTERS,
  MAX_DOCUMENT,
  MAX_MISTAKES,
  MAX_STUDIES,
  SESSION_KINDS,
  decodeArchive,
  decodeJoinedTournaments,
  decodeMistakes,
  decodeRepertoireMisses,
  decodeStoredSession,
  decodeStudies,
  encodeSession,
  studyContent,
  studyDocumentPgn,
  type ArchivedGame,
  type JoinedTournament,
  type LegacyDocuments,
  type LibrarySnapshot,
  type MistakeExercise,
  type RepertoireMisses,
  type SavedStudy,
  type SessionDocuments,
  type SessionKind,
  type StudyCommand,
  type StudyCommandResult,
} from '../shared/library'
import type { TournamentSystem } from '../shared/types'
import { getDb } from './db'
import { logDebug, logWarn } from './logger'
import { readReview } from './reviewStore'

/* ── Documents: one bounded JSON value per key ── */

function readDocument(key: string): unknown {
  const row = getDb().prepare('SELECT body FROM documents WHERE key = ?').get(key) as
    { body: string } | undefined
  if (!row) return undefined
  try {
    return JSON.parse(row.body)
  } catch (cause) {
    logWarn('library', 'Stored document is not valid JSON:', `key=${key}`, cause)
    return undefined
  }
}

function writeDocument(key: string, value: unknown, tooLarge: string): void {
  const body = JSON.stringify(value)
  if (body.length > MAX_DOCUMENT) throw new Error(tooLarge)
  getDb()
    .prepare(
      `INSERT INTO documents (key, body, updatedAt) VALUES (?, ?, ?)
       ON CONFLICT(key) DO UPDATE SET body = excluded.body, updatedAt = excluded.updatedAt`,
    )
    .run(key, body, Date.now())
}

const STUDIES = 'studies'
const GAMES = 'games'
const MISTAKES = 'mistakes'
const JOINED = 'tournaments:joined'
const MISSES = 'repertoire:misses'
const IMPORTED = 'library:imported'
const session = (kind: SessionKind) => `session:${kind}`

/* ── Snapshot and the one-time import from an earlier release ── */

/** Whether the library already has this document (or, for games, any game). */
function stored(key: string): boolean {
  if (key === GAMES)
    return getDb().prepare('SELECT 1 FROM archived_games LIMIT 1').get() !== undefined
  return readDocument(key) !== undefined
}

const readStudies = (): SavedStudy[] => decodeStudies(readDocument(STUDIES)) ?? []
const readMistakes = (): MistakeExercise[] => decodeMistakes(readDocument(MISTAKES)) ?? []
const readMisses = (): RepertoireMisses => decodeRepertoireMisses(readDocument(MISSES))
const readJoined = (): JoinedTournament[] => decodeJoinedTournaments(readDocument(JOINED))

export function readSession<K extends SessionKind>(kind: K): SessionDocuments[K] | undefined {
  return decodeStoredSession(kind, readDocument(session(kind)))
}

export function library(now = Date.now()): LibrarySnapshot {
  const sessions: Partial<SessionDocuments> = {}
  for (const kind of SESSION_KINDS) {
    const value = readSession(kind)
    if (value) (sessions as Record<SessionKind, unknown>)[kind] = value
  }
  return {
    studies: readStudies(),
    games: readGames(),
    mistakes: readMistakes(),
    sessions,
    joinedTournaments: readJoined().filter((entry) => entry.until > now),
    repertoireMisses: readMisses(),
    imported: readDocument(IMPORTED) === true,
  }
}

const LEGACY_TARGETS: Record<(typeof LEGACY_DOCUMENT_KEYS)[number], string> = {
  'kchess:studies:v1': STUDIES,
  'kchess:game-history:v1': GAMES,
  'kchess:mistakes:v1': MISTAKES,
  'kchess:analysis:v1': session('analysis'),
  'kchess:local:v1': session('local'),
  'kchess:computer:v1': session('computer'),
  'kchess:history-session:computer': session('archive:computer'),
  'kchess:history-session:board': session('archive:board'),
  'kchess:history-session:clock': session('archive:clock'),
  'kchess:tournaments-joined': JOINED,
  'kchess:repertoire-misses': MISSES,
}
const LEGACY_DECODERS: Record<string, (raw: unknown) => unknown> = {
  [STUDIES]: (raw) => {
    const studies = decodeStudies(raw)
    return studies && { version: 2, items: studies }
  },
  [GAMES]: (raw) => decodeArchive(raw),
  [MISTAKES]: (raw) => {
    const items = decodeMistakes(raw)
    return items && { version: 1, items }
  },
  [JOINED]: (raw) => decodeJoinedTournaments(raw),
  [MISSES]: (raw) => decodeRepertoireMisses(raw),
  ...Object.fromEntries(
    SESSION_KINDS.map((kind) => [
      session(kind),
      (raw: unknown) => {
        const value = decodeStoredSession(kind, raw)
        return value && encodeSession(kind, value)
      },
    ]),
  ),
}

/**
 * Take over documents an earlier release kept in the renderer, once. Each is validated as the
 * library would; invalid ones are skipped. Nothing already in the library is replaced.
 */
export function importLibrary(documents: LegacyDocuments): LibrarySnapshot {
  if (readDocument(IMPORTED) === true) return library()
  const database = getDb()
  database.exec('BEGIN IMMEDIATE')
  try {
    for (const legacyKey of LEGACY_DOCUMENT_KEYS) {
      const text = documents[legacyKey]
      const key = LEGACY_TARGETS[legacyKey]
      if (typeof text !== 'string' || text.length > MAX_DOCUMENT || stored(key)) continue
      let value: unknown
      try {
        value = LEGACY_DECODERS[key]!(JSON.parse(text))
      } catch (cause) {
        logWarn('library', 'Skipped an unreadable saved document:', `key=${legacyKey}`, cause)
        continue
      }
      if (value === undefined) {
        logWarn('library', 'Skipped an invalid saved document:', `key=${legacyKey}`)
        continue
      }
      if (key === GAMES) {
        // Oldest first, so the most recent game ends up most recent again.
        for (const game of [...(value as ArchivedGame[])].reverse()) writeGame(game)
        continue
      }
      writeDocument(key, value, 'A saved document is too large to import.')
    }
    writeDocument(IMPORTED, true, '')
    database.exec('COMMIT')
  } catch (cause) {
    database.exec('ROLLBACK')
    throw cause
  }
  return library()
}

/* ── Studies ── */

const clean = (name: string) => name.trim().slice(0, 120)
const valid = (pgn: string) => Boolean(treeFromPgn(pgn))

function writeStudies(studies: SavedStudy[]): void {
  writeDocument(
    STUDIES,
    { version: 2, items: studies },
    'This study is too large to save automatically. Export a PGN copy.',
  )
}

/** Apply one change to the study library and store it; the library is unchanged on failure. */
export function studyCommand(command: StudyCommand, now = Date.now()): StudyCommandResult {
  let items = readStudies()
  const find = (id: string) => items.find((item) => item.id === id)
  function commit(candidate: SavedStudy): string {
    const next = [candidate, ...items.filter((item) => item.id !== candidate.id)]
    if (next.length > MAX_STUDIES || JSON.stringify(next).length > 1_800_000)
      throw new Error('The study library is full. Export and remove a study before saving another.')
    items = next
    return candidate.id
  }
  function saveChapters(
    rawName: string,
    chapters: { name: string; pgn: string }[],
    id?: string,
  ): string {
    const name = clean(rawName)
    if (
      !name ||
      !chapters.length ||
      chapters.length > MAX_CHAPTERS ||
      !chapters.every((c) => valid(c.pgn))
    )
      throw new Error('Name the study and use up to 64 valid chapters.')
    const old = id ? find(id) : undefined
    const saved = chapters.map((c, index) => ({
      id: old?.chapters[index]?.id ?? randomUUID(),
      name: c.name.slice(0, 120),
      pgn: c.pgn,
    }))
    return commit({
      id: id ?? randomUUID(),
      name,
      pgn: saved[0]!.pgn,
      chapters: saved,
      updatedAt: now,
      ...(old?.cloud ? { cloud: old.cloud } : {}),
    })
  }
  const result: Omit<StudyCommandResult, 'studies'> = {}
  switch (command.op) {
    case 'saveChapters':
      result.id = saveChapters(command.name, command.chapters, command.id)
      break
    case 'save': {
      const name = clean(command.name)
      if (!name) throw new Error('Name the study.')
      const old = command.id ? find(command.id) : undefined
      if (!old) {
        result.id = saveChapters(name, [{ name, pgn: command.pgn }], command.id)
        break
      }
      const index = command.chapterId
        ? old.chapters.findIndex((c) => c.id === command.chapterId)
        : 0
      if (index < 0 || !valid(command.pgn)) throw new Error('That chapter cannot be saved.')
      const chapters = old.chapters.map((c, i) => (i === index ? { ...c, pgn: command.pgn } : c))
      result.id = commit({ ...old, name, chapters, pgn: chapters[0]!.pgn, updatedAt: now })
      break
    }
    case 'addChapter': {
      const study = find(command.id)
      if (!study || !command.name.trim()) throw new Error('Name the new chapter.')
      saveChapters(
        study.name,
        [...study.chapters, { name: command.name.trim(), pgn: '*' }],
        study.id,
      )
      result.id = find(study.id)!.chapters.at(-1)!.id
      break
    }
    case 'renameChapter': {
      const study = find(command.id)
      if (!study || !command.name.trim()) break
      const chapters = study.chapters.map((c) =>
        c.id === command.chapterId ? { ...c, name: clean(command.name) } : c,
      )
      commit({ ...study, chapters, updatedAt: now })
      break
    }
    case 'duplicateChapter': {
      const study = find(command.id)
      const original = study?.chapters.find((c) => c.id === command.chapterId)
      if (!study || !original) throw new Error('That chapter no longer exists.')
      if (study.chapters.length >= MAX_CHAPTERS)
        throw new Error('A study can contain up to 64 chapters.')
      const chapter = {
        ...original,
        id: randomUUID(),
        name: `${original.name.slice(0, 113)} (copy)`,
      }
      commit({ ...study, chapters: [...study.chapters, chapter], updatedAt: now })
      result.id = chapter.id
      break
    }
    case 'removeChapter': {
      const study = find(command.id)
      const index = study?.chapters.findIndex((c) => c.id === command.chapterId) ?? -1
      if (!study || index < 0) throw new Error('That chapter no longer exists.')
      if (study.chapters.length === 1) throw new Error('A study must keep at least one chapter.')
      const chapters = study.chapters.filter((c) => c.id !== command.chapterId)
      commit({
        ...study,
        chapters,
        pgn: chapters[0]!.pgn,
        updatedAt: now,
        ...(study.cloud ? { cloud: { ...study.cloud, structureChanged: true } } : {}),
      })
      result.id = chapters[Math.min(index, chapters.length - 1)]!.id
      break
    }
    case 'markCloud': {
      const study = find(command.id)
      if (!study) break
      items = items.map((item) =>
        item === study
          ? {
              ...item,
              cloud: {
                account: command.account,
                id: command.remoteId,
                downloadedPgn: command.baseline,
              },
            }
          : item,
      )
      break
    }
    case 'offline': {
      const { account, remote, chapters } = command
      const old = items.find(
        (s) => s.cloud?.account.toLowerCase() === account.toLowerCase() && s.cloud.id === remote.id,
      )
      // A changed device copy is kept; the cloud version then becomes a separate copy.
      const conflict = Boolean(
        old && studyContent(studyDocumentPgn(old)) !== studyContent(old.cloud?.downloadedPgn ?? ''),
      )
      const id = saveChapters(
        conflict ? `${remote.name} (cloud copy)` : remote.name,
        chapters,
        conflict ? undefined : old?.id,
      )
      const downloadedPgn = chapters.map((c) => c.pgn).join('\n\n')
      items = items.map((item) =>
        item.id === id ? { ...item, cloud: { account, id: remote.id, downloadedPgn } } : item,
      )
      result.id = id
      result.conflict = conflict
      break
    }
    case 'remove': {
      const removed = find(command.id)
      items = items.filter((item) => item.id !== command.id)
      if (removed) result.removed = removed
      break
    }
    case 'restore':
      if (!find(command.study.id)) commit(command.study)
      break
    case 'rename': {
      const study = find(command.id)
      if (study && command.name.trim())
        commit({ ...study, name: clean(command.name), updatedAt: now })
      break
    }
    case 'duplicate': {
      const study = find(command.id)
      if (study) result.id = saveChapters(`${study.name.slice(0, 113)} (copy)`, study.chapters)
      break
    }
  }
  writeStudies(items)
  return { studies: items, ...result }
}

/* ── Played games: one row each, so a move rewrites only its own game ── */

const GAMES_FULL = 'Game history is full. Export saved games before making room for new games.'
/** The size the archive had as one document, so its limit stays what it was. */
const ARCHIVE_OVERHEAD = JSON.stringify({ version: 1, games: [] }).length

function readGames(): ArchivedGame[] {
  const rows = getDb()
    .prepare('SELECT id, body FROM archived_games ORDER BY seq DESC')
    .all() as unknown as { id: string; body: string }[]
  return rows.flatMap((row) => {
    try {
      const game = decodeArchivedGame(JSON.parse(row.body))
      if (game) return [game]
    } catch (cause) {
      logWarn('library', 'Archived game is not valid JSON:', `id=${row.id}`, cause)
    }
    return []
  })
}

function writeGame(game: ArchivedGame): void {
  const database = getDb()
  const body = JSON.stringify(game)
  const { count, bytes } = database
    .prepare(
      'SELECT COUNT(*) AS count, COALESCE(SUM(length(body)), 0) AS bytes FROM archived_games WHERE id != ?',
    )
    .get(game.id) as unknown as { count: number; bytes: number }
  // Never silently discard older games to make room for a new one.
  if (
    count + 1 > MAX_ARCHIVED_GAMES ||
    ARCHIVE_OVERHEAD + bytes + count + body.length > MAX_DOCUMENT
  )
    throw new Error(GAMES_FULL)
  database
    .prepare(
      `INSERT INTO archived_games (id, body, seq)
       VALUES (?, ?, (SELECT COALESCE(MAX(seq), 0) + 1 FROM archived_games))
       ON CONFLICT(id) DO UPDATE SET body = excluded.body, seq = excluded.seq`,
    )
    .run(game.id, body)
}

/** Add or replace a game; it becomes the most recent. */
export function saveArchivedGame(game: ArchivedGame): void {
  writeGame(game)
}

export function removeArchivedGame(id: string): void {
  getDb().prepare('DELETE FROM archived_games WHERE id = ?').run(id)
}

/* ── Mistake drills ── */

function writeMistakes(items: MistakeExercise[]): void {
  writeDocument(MISTAKES, { version: 1, items }, 'Too many mistakes are saved to keep more.')
}

/** Turn a completed review's mistakes (optionally one side's) into drills; returns how many were new. */
export function addMistakes(
  reviewKey: string,
  color?: 'white' | 'black',
  now = Date.now(),
): { added: number; items: MistakeExercise[] } {
  const review = readReview(reviewKey)
  let items = readMistakes()
  if (!review?.complete) return { added: 0, items }
  const positions = replay(review.fen, review.moves)
  const analysis = analyseReview(review)
  let added = 0
  for (const [index, move] of analysis.moves.entries()) {
    if (!move.judgment || (color && move.color !== color)) continue
    const evaluation = review.evals[index]
    const best = evaluation?.best
    const fen = positions[index]?.fen
    if (
      !best ||
      !fen ||
      best === review.moves[index] ||
      (review.source === 'local' && (evaluation.depth ?? 0) < 10)
    )
      continue
    let solution = evaluation.pv?.[0] === best ? evaluation.pv.slice(0, 7) : [best]
    solution = solution.slice(0, replay(fen, solution).length - 1)
    if (solution.length % 2 === 0) solution.pop()
    if (!solution.length) continue
    const id = `${review.key}:${index}`
    if (items.some((item) => item.id === id)) continue
    items.push({ id, fen, solution, judgment: move.judgment, dueAt: now, streak: 0, attempts: 0 })
    added++
  }
  items = items.slice(-MAX_MISTAKES)
  if (added) writeMistakes(items)
  return { added, items }
}

/** Schedule a drill again: soon after a miss, further out after each clean solve. */
export function answerMistake(id: string, solved: boolean, now = Date.now()): MistakeExercise[] {
  const items = readMistakes()
  const item = items.find((entry) => entry.id === id)
  if (!item) return items
  item.attempts++
  item.streak = solved ? item.streak + 1 : 0
  item.dueAt = now + (solved ? Math.min(30, 2 ** (item.streak - 1)) * 86_400_000 : 10 * 60_000)
  writeMistakes(items)
  return items
}

/* ── Unfinished sessions ── */

export function saveSession<K extends SessionKind>(kind: K, value: SessionDocuments[K]): void {
  writeDocument(
    session(kind),
    encodeSession(kind, value),
    'This session is too large to save automatically. Export a PGN copy.',
  )
}

/* ── Tournaments joined from KChess ── */

function writeJoined(entries: JoinedTournament[]): void {
  writeDocument(JOINED, entries, 'Too many joined tournaments.')
}

export function joinedTournaments(now = Date.now()): JoinedTournament[] {
  return readJoined().filter((entry) => entry.until > now)
}

export function rememberTournament(entry: JoinedTournament, now = Date.now()): void {
  writeJoined([
    ...readJoined().filter(
      (e) => e.until > now && !(e.system === entry.system && e.id === entry.id),
    ),
    entry,
  ])
}

export function forgetTournament(system: TournamentSystem, id: string): void {
  writeJoined(readJoined().filter((e) => !(e.system === system && e.id === id)))
}

/** Signed-out accounts must not reopen event streams for their tournaments. */
export function forgetTournamentsOf(accounts: readonly string[]): void {
  const names = new Set(accounts.map((name) => name.toLowerCase()))
  const entries = readJoined()
  const kept = entries.filter((e) => !names.has(e.account.toLowerCase()))
  if (kept.length !== entries.length) {
    writeJoined(kept)
    logDebug(
      'library',
      'Forgot joined tournaments of signed-out accounts:',
      `count=${entries.length - kept.length}`,
    )
  }
}

/* ── Repertoire training ── */

export function recordRepertoireMiss(key: string, fen: string): RepertoireMisses {
  const misses = readMisses()
  const table = (misses[key] ??= {})
  table[fen] = (table[fen] ?? 0) + 1
  writeDocument(MISSES, misses, 'Too many repertoire notes are saved.')
  return misses
}

export function clearRepertoireMisses(key: string): RepertoireMisses {
  const misses = readMisses()
  delete misses[key]
  writeDocument(MISSES, misses, 'Too many repertoire notes are saved.')
  return misses
}
