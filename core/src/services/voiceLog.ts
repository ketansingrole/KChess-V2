import { nativeCallSync } from './nativeCore'
import type { VoiceAttempt, VoiceAttemptInput, VoiceAttemptUpdate } from '../contracts/types'

/**
 * Voice recognition attempts, kept in the Rust core's `kchess.db` (`crates/kchess-core/src/store/voice_log.rs`),
 * which trims the log to its newest entries.
 */

export function saveVoiceAttempt(attempt: VoiceAttemptInput): number {
  return nativeCallSync<number>('store.voiceLog.saveVoiceAttempt', attempt)
}

export function updateVoiceAttempt(id: number, update: VoiceAttemptUpdate): void {
  nativeCallSync('store.voiceLog.updateVoiceAttempt', id, update)
}

export function voiceHistory(limit: number): VoiceAttempt[] {
  return nativeCallSync<VoiceAttempt[]>('store.voiceLog.voiceHistory', limit)
}

export function clearVoiceHistory(): void {
  nativeCallSync('store.voiceLog.clearVoiceHistory')
}

/** The whole log as a JSON document, for export. */
export function voiceHistoryDocument(): string {
  return nativeCallSync<string>('store.voiceLog.voiceHistoryDocument')
}
