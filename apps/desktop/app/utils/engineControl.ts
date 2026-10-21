import type { BestMoveOptions, EngineLevel } from '@kchess/contracts/types'

/**
 * One logged `stopAnalysis` call, so engine teardown stays wired to the
 * main-owned UCI search from every caller.
 */
export function stopAnalysisSafe(scope: string, release?: boolean): void {
  void window.kchess.stopAnalysis(release).catch((error: unknown) => {
    console.warn(`[${scope}] stopping analysis failed:`, error)
  })
}

/** The core runs one computer search at a time; each `bestMove` supersedes the one running. */
let latestSearch = 0
let searchRunning = false

/**
 * `bestMove` with a renderer-wide ticket, so a caller stops only its own search and never one a
 * later caller (another view, the computer game) started since.
 */
export function bestMoveOwned(
  moves: string[],
  level: EngineLevel,
  options: BestMoveOptions,
): { ticket: number; result: Promise<string> } {
  const ticket = ++latestSearch
  searchRunning = true
  const result = window.kchess.bestMove(moves, level, options).finally(() => {
    if (ticket === latestSearch) searchRunning = false
  })
  return { ticket, result }
}

/** Stop the core's search if it is still this ticket's. */
export function stopOwnedSearch(scope: string, ticket: number): void {
  if (ticket !== latestSearch || !searchRunning) return
  searchRunning = false
  void window.kchess.stopEngine().catch((error: unknown) => {
    console.warn(`[${scope}] stopping the engine failed:`, error)
  })
}
