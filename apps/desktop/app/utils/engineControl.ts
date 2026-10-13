/**
 * One logged `stopAnalysis` call, so engine teardown stays wired to the
 * main-owned UCI search from every caller.
 */
export function stopAnalysisSafe(scope: string, release?: boolean): void {
  void window.kchess.stopAnalysis(release).catch((error: unknown) => {
    console.warn(`[${scope}] stopping analysis failed:`, error)
  })
}
