/**
 * The pending challenges of the connected accounts. The Rust core keeps the inbox (it reads each
 * challenge event as it arrives and emits `challenges:update` with the whole list); these
 * wrappers read and change it.
 */
import type { ChallengeInfo } from '../contracts/types'
import { rules } from '../domain/engine'
import { nativeCallSync } from './nativeCore'

/** A challenge event from an account's stream, seen from that account (`readChallengeEvent`). */
export function readChallengeEvent(
  account: string,
  raw: unknown,
  now: number,
):
  | { kind: 'challenge' | 'challengeCanceled' | 'challengeDeclined'; info: ChallengeInfo }
  | undefined {
  return (
    rules<{
      kind: 'challenge' | 'challengeCanceled' | 'challengeDeclined'
      info: ChallengeInfo
    } | null>('readChallengeEvent', account, raw, now) ?? undefined
  )
}

export class ChallengeInbox {
  /** The open challenges, newest first. */
  list(): ChallengeInfo[] {
    return nativeCallSync('challenges.list')
  }

  get(id: string): ChallengeInfo | undefined {
    return nativeCallSync<ChallengeInfo | null>('challenges.get', id) ?? undefined
  }

  remove(id: string): void {
    nativeCallSync('challenges.remove', id)
  }

  /** Forgets the challenges of an account that was signed out or removed. */
  forget(account: string): void {
    nativeCallSync('challenges.forget', account)
  }
}
