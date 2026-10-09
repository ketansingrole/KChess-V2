import type { VoiceAttemptInput, VoiceAttemptUpdate } from '../../../../core/src/contracts/types'
import { useKChessStore } from '../stores/kchess'
import type { VoiceResult } from './voiceCapture'

/** The logged part of a recognizer result: the raw text (with [unk]) and per-word confidences. */
export function heardFields(
  result: VoiceResult,
): Pick<VoiceAttemptInput, 'heard' | 'confidence' | 'words'> {
  return {
    heard: (result.raw ?? result.text).slice(0, 200),
    confidence: Math.max(0, Math.min(1, Math.round(result.confidence * 1000) / 1000)),
    words: (result.words ?? []).slice(0, 40).map((word) => ({
      word: word.word.slice(0, 40),
      conf: Math.max(0, Math.min(1, word.conf)),
    })),
  }
}

/** Record one phrase in the local voice history (Settings › Voice); a no-op when that is off. */
export async function logVoice(attempt: VoiceAttemptInput): Promise<number | undefined> {
  if (!useKChessStore().settings?.voiceHistory) return undefined
  try {
    return await window.kchess.saveVoiceAttempt({
      ...attempt,
      parsed: attempt.parsed?.slice(0, 120),
    })
  } catch (cause) {
    console.warn('[voice-log] saving voice attempt failed:', cause)
    // The history is a study aid; losing an entry must never disturb play.
    return undefined
  }
}

/** Record what came of a logged phrase, once it is known. */
export function updateVoice(id: number | undefined, update: VoiceAttemptUpdate): void {
  if (!id) return
  void window.kchess.updateVoiceAttempt(id, update).catch((error: unknown) => {
    console.warn('[voice-log] updating voice attempt failed:', error)
  })
}
