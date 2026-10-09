import type { OnlineEvent } from '../contracts/types'
import { rules } from './engine.ts'

/**
 * Unknown event types can be ignored; malformed recognized game state is a recoverable error.
 * The schema and its messages are the Rust rules' (`crates/kchess-domain/src/records/online.rs`).
 */
export function validateOnlineEvent(raw: unknown): OnlineEvent | undefined {
  return rules<OnlineEvent | null>('validateOnlineEvent', raw) ?? undefined
}
