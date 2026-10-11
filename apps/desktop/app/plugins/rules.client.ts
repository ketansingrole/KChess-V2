import rulesUrl from '../assets/rules/kchess.wasm?url'
import { setRulesBinding, wasmBinding } from '@kchess/rules/engine'
import { instantiateRules } from '../utils/loadRules'

/** The chess rules (Rust, as WebAssembly) load before anything that plays or reads a move. */
export default defineNuxtPlugin({
  name: 'rules',
  enforce: 'pre',
  async setup() {
    const response = await fetch(rulesUrl)
    const instance = await instantiateRules(response)
    setRulesBinding(wasmBinding(instance.exports))
  },
})
