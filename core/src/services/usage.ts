/**
 * Network accounting for everything KChess asks of Lichess. The Rust core counts the requests and
 * bytes of each account and kind, batches them in memory and writes them to `kchess.db` every few
 * seconds (`crates/kchess-core/src/usage.rs`); these wrappers flush, report and reset them.
 */
import type { UsageReport } from '../contracts/types'
import { nativeCallSync } from './nativeCore'

/** Write the counters accumulated in memory to the database. */
export function flushUsage(): void {
  nativeCallSync('usage.flush')
}

/** Ignore traffic still arriving from requests started before logout. */
export function forgetUsage(accounts: string[]): void {
  nativeCallSync('usage.forget', accounts)
}

export function resetUsage(): void {
  nativeCallSync('usage.reset')
}

/** What each account has downloaded from Lichess and what it occupies on this computer. */
export async function usageReport(): Promise<UsageReport> {
  return nativeCallSync('usage.report')
}

/** Stops the batching: pending counters are dropped (the core is closing). */
export function closeUsage(): void {
  nativeCallSync('usage.close')
}
