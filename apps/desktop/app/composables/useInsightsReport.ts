import { onScopeDispose, type ComputedRef } from 'vue'
import { RequestScope } from '../../../../core/src/domain/requestScope'
import type { InsightsQuery } from '../../../../core/src/contracts/types'
import { requestInsights } from '../utils/insightsRequest'

export function useInsightsReport(query: ComputedRef<InsightsQuery>) {
  const scope = new RequestScope()
  const result = useAsyncData(
    'insights-report',
    (_app, { signal }) => requestInsights(scope, { ...query.value }, signal),
    {
      server: false,
      lazy: true,
      deep: false,
      dedupe: 'cancel',
      enabled: () => Boolean(query.value.account),
      watch: [query],
      default: () => null,
      getCachedData: () => undefined,
    },
  )
  onScopeDispose(() => {
    scope.invalidate()
    result.clear()
  })
  return result
}
