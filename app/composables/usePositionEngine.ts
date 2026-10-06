import { onScopeDispose, ref, shallowRef, watch } from 'vue'
import type { AnalysisRequest, AnalysisUpdate } from '../../src/shared/types'
import { analysisContext } from '../../src/shared/analysisContext'
import { RequestScope } from '../../src/shared/requestScope'

let nextClient = Date.now()

/** Own one visible position's engine search, without changing saved analysis or studies. */
export function usePositionEngine(request: () => AnalysisRequest | null) {
  const update = shallowRef<AnalysisUpdate | null>(null)
  const error = ref('')
  const busy = ref(false)
  const requests = new RequestScope()
  let wanted: AnalysisRequest | null = null
  let clientId = 0
  let timer: ReturnType<typeof setTimeout> | undefined
  const off = window.kchess.onAnalysis((value) => {
    if (
      !wanted ||
      value.clientId !== clientId ||
      value.fen !== wanted.fen ||
      value.context !== analysisContext(wanted.rootFen ?? wanted.fen, wanted.moves)
    )
      return
    update.value = value.error ? null : value
    error.value = value.error ?? ''
    busy.value = !value.done
  })
  watch(
    request,
    (value) => {
      const current = requests.next()
      clientId = ++nextClient
      wanted = value
      update.value = null
      error.value = ''
      busy.value = Boolean(value)
      clearTimeout(timer)
      // Cancellation reaches the main-owned UCI search, including when the switch turns off.
      const stopped = window.kchess.stopAnalysis().catch(() => undefined)
      if (!value) return
      const id = clientId
      timer = setTimeout(async () => {
        await stopped
        if (!current.current()) return
        try {
          await window.kchess.startAnalysis({ ...value, clientId: id })
        } catch (cause) {
          if (!current.current()) return
          error.value = cause instanceof Error ? cause.message : String(cause)
          busy.value = false
        }
      }, 100)
    },
    { immediate: true, flush: 'sync' },
  )
  onScopeDispose(() => {
    clearTimeout(timer)
    requests.invalidate()
    wanted = null
    off()
    void window.kchess.stopAnalysis().catch(() => undefined)
  })
  return { update, error, busy }
}
