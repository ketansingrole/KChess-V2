import { nativeCall, nativeCallSync, onNativeEvent } from './nativeCore.ts'
import { scopedState } from './platform.ts'
import type { AnalysisUpdate } from '../contracts/types'

/**
 * Infinite and bounded analysis, run by the Rust core (`crates/kchess-core/src/engine/analysis.rs`).
 * The core streams `engine:analysis` updates; they reach the sink of the latest `startAnalysis`.
 */

const forwarding = scopedState(() => ({
  unsubscribe: undefined as (() => void) | undefined,
  send: undefined as ((update: AnalysisUpdate) => void) | undefined,
}))

export async function startAnalysis(
  raw: unknown,
  configured: string,
  send: (update: AnalysisUpdate) => void,
): Promise<number> {
  forwarding.send = send
  forwarding.unsubscribe?.()
  forwarding.unsubscribe = onNativeEvent<AnalysisUpdate>('engine:analysis', (update) =>
    forwarding.send?.(update),
  )
  return nativeCall<number>('analysis.start', raw, configured)
}

export const analysisRunning = (): boolean => nativeCallSync<boolean>('analysis.running')

export function stopAnalysis(kill = false): void {
  nativeCallSync('analysis.stop', kill)
}
