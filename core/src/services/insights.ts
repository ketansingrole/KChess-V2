import { nativeCallSync } from './nativeCore'
import type { InsightsQuery, InsightsReport } from '../contracts/types'

/**
 * Patterns in the games synced to this computer: results by colour, speed, opening, time of
 * day, opponent strength and game length, and accuracy where games were reviewed. Everything is
 * read locally from the Rust core's `kchess.db` (`crates/kchess-core/src/store/insights.rs`);
 * nothing is sent to Lichess.
 */
export function insights(query: InsightsQuery): InsightsReport {
  return nativeCallSync<InsightsReport>('store.insights.insights', query)
}
