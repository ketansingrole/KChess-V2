import { dialog } from 'electron'
import { writeFile } from 'node:fs/promises'
import { getDb } from './db'
import { logDebug } from './logger'
import type {
  VoiceAttempt,
  VoiceAttemptInput,
  VoiceAttemptUpdate,
  VoiceOutcome,
  VoiceSource,
  VoiceWord,
} from '../shared/types'

/** Oldest entries beyond this are dropped; enough to study, small enough to never matter on disk. */
const MAX_ENTRIES = 5000

interface VoiceRow {
  id: number
  at: number
  source: string
  heard: string
  confidence: number
  words: string
  outcome: string
  parsed: string | null
  expected: string | null
  fen: string | null
  retryOf: number | null
}

function toAttempt(row: VoiceRow): VoiceAttempt {
  let words: VoiceWord[] = []
  try {
    words = JSON.parse(row.words) as VoiceWord[]
  } catch (cause) {
    logDebug('voice', 'Voice log row has invalid words JSON:', cause)
  }
  return {
    id: row.id,
    at: row.at,
    source: row.source as VoiceSource,
    heard: row.heard,
    confidence: row.confidence,
    words,
    outcome: row.outcome as VoiceOutcome,
    parsed: row.parsed ?? undefined,
    expected: row.expected ?? undefined,
    fen: row.fen ?? undefined,
    retryOf: row.retryOf ?? undefined,
  }
}

export function saveVoiceAttempt(attempt: VoiceAttemptInput): number {
  const database = getDb()
  const { lastInsertRowid } = database
    .prepare(
      `INSERT INTO voice_log (at, source, heard, confidence, words, outcome, parsed, expected, fen, retryOf)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    )
    .run(
      Date.now(),
      attempt.source,
      attempt.heard,
      attempt.confidence,
      JSON.stringify(attempt.words),
      attempt.outcome,
      attempt.parsed ?? null,
      attempt.expected ?? null,
      attempt.fen ?? null,
      attempt.retryOf ?? null,
    )
  database
    .prepare(
      `DELETE FROM voice_log WHERE id NOT IN (SELECT id FROM voice_log ORDER BY id DESC LIMIT ${MAX_ENTRIES})`,
    )
    .run()
  return Number(lastInsertRowid)
}

export function updateVoiceAttempt(id: number, update: VoiceAttemptUpdate): void {
  const database = getDb()
  if (update.outcome)
    database.prepare('UPDATE voice_log SET outcome = ? WHERE id = ?').run(update.outcome, id)
  if (update.expected)
    database.prepare('UPDATE voice_log SET expected = ? WHERE id = ?').run(update.expected, id)
}

export function voiceHistory(limit: number): VoiceAttempt[] {
  return (
    getDb()
      .prepare('SELECT * FROM voice_log ORDER BY id DESC LIMIT ?')
      .all(limit) as unknown as VoiceRow[]
  ).map(toAttempt)
}

export function clearVoiceHistory(): void {
  getDb().exec('DELETE FROM voice_log')
}

export async function exportVoiceHistory(): Promise<boolean> {
  const date = new Date().toISOString().slice(0, 10)
  const result = await dialog.showSaveDialog({
    title: 'Export voice history',
    defaultPath: `kchess-voice-history-${date}.json`,
    filters: [{ name: 'JSON', extensions: ['json'] }],
  })
  if (result.canceled || !result.filePath) return false
  const entries = voiceHistory(MAX_ENTRIES).map((entry) => ({
    ...entry,
    time: new Date(entry.at).toISOString(),
  }))
  await writeFile(
    result.filePath,
    `${JSON.stringify({ exportedAt: Date.now(), entries }, null, 2)}\n`,
  )
  return true
}
