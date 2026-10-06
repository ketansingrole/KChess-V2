import { diagnosticRoute } from '../../src/shared/rendererDiagnostics'

export default defineNuxtPlugin((nuxtApp) => {
  const router = useRouter()
  let started: number | undefined
  nuxtApp.hook('page:loading:start', () => {
    started = performance.now()
  })
  nuxtApp.hook('page:loading:end', () => {
    if (started === undefined) return
    const elapsed = performance.now() - started
    started = undefined
    void window.kchess
      ?.recordPerformance(
        `page.navigation:${diagnosticRoute(router.currentRoute.value.path)}`,
        elapsed,
      )
      .catch(() => {})
  })
  nuxtApp.hook('vue:error', (error, _instance, info) => {
    void window.kchess
      ?.reportRendererError({
        route: diagnosticRoute(router.currentRoute.value.path),
        message: (error instanceof Error ? error.message : String(error)).slice(0, 1000),
        info: info.slice(0, 160),
      })
      .catch(() => {})
  })
})
