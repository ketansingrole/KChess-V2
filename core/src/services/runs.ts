import { nativeCallSync } from './nativeCore'
import type { RunInput, RunKind, RunSaved, RunSummary } from '../contracts/types'

/**
 * Local scores for Storm, Streak, Rush and the practice drills, stored in the Rust core's `kchess.db`
 * (`crates/kchess-core/src/store/runs.rs`). Neither is ever sent anywhere.
 */

export function runSummary(kind: RunKind): RunSummary {
  return nativeCallSync<RunSummary>('store.runs.runSummary', kind)
}

export function saveRun(run: RunInput): RunSaved {
  return nativeCallSync<RunSaved>('store.runs.saveRun', run)
}

/** Forget the local scores of one kind, or of all of them. */
export function clearRuns(kind?: RunKind): void {
  nativeCallSync('store.runs.clearRuns', kind ?? null)
}
