import type { RequestScope } from '@kchess/rules/requestScope'
import type { InsightsQuery, InsightsReport } from '@kchess/contracts/types'

/** Insights is a bounded synchronous SQLite read in main, with no remote work to cancel. */
export async function requestInsights(
  scope: RequestScope,
  query: InsightsQuery,
  signal: AbortSignal,
  read: (query: InsightsQuery) => Promise<InsightsReport> = (query) =>
    window.kchess.insights(query),
): Promise<InsightsReport> {
  const request = scope.next()
  const abort = () => {
    if (request.current()) scope.invalidate()
  }
  if (signal.aborted) throw new DOMException('Insights request cancelled', 'AbortError')
  signal.addEventListener('abort', abort, { once: true })
  try {
    const report = await read(query)
    if (!request.current() || signal.aborted)
      throw new DOMException('Insights request cancelled', 'AbortError')
    return report
  } catch (error) {
    if (!request.current() || signal.aborted)
      throw new DOMException('Insights request cancelled', 'AbortError')
    throw error
  } finally {
    signal.removeEventListener('abort', abort)
  }
}
