import { readonly, ref } from 'vue'
import type { InterfaceZoom, InterfaceZoomState } from '../../contracts/interfaceZoom'

const state = ref<InterfaceZoomState>({ setting: 'auto', percent: 100, autoPercent: 100 })

/**
 * Natively drawn chrome (the macOS traffic lights) does not zoom with the page; CSS that sits
 * beside it divides by `--interface-zoom` to keep a constant on-screen distance.
 */
function show(next: InterfaceZoomState): void {
  state.value = next
  document.documentElement.style.setProperty('--interface-zoom', String(next.percent / 100))
}

/** Follows the window's zoom from the start, before the workspace mounts. */
export function startInterfaceZoom(): void {
  const api = window.kchess
  if (!api?.interfaceZoom) return
  api.onInterfaceZoom(show)
  void api
    .interfaceZoom()
    .then(show)
    .catch((error: unknown) => console.warn('[interface-zoom] Reading the zoom failed:', error))
}

export function useInterfaceZoom() {
  return {
    zoom: readonly(state),
    async setZoom(setting: InterfaceZoom): Promise<void> {
      show(await window.kchess.setInterfaceZoom(setting))
    },
  }
}
