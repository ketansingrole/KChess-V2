import rulesUrl from '../assets/rules/kchess.wasm?url'
import { setRulesBinding, wasmBinding } from '@kchess/core/domain/engine'

/** The chess rules (Rust, as WebAssembly) load before anything that plays or reads a move. */
export default defineNuxtPlugin({
  name: 'rules',
  enforce: 'pre',
  async setup() {
    const response = await fetch(rulesUrl)
    if (!response.ok) throw new Error(`The chess rules could not be loaded (${response.status}).`)
    const { instance } = await WebAssembly.instantiate(await response.arrayBuffer())
    setRulesBinding(wasmBinding(instance.exports))
  },
})
