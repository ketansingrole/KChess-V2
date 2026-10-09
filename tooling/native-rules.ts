import { createRequire } from 'node:module'
import { setRulesBinding } from '../core/src/domain/engine.ts'
import type { NativeRules } from '../core/src/services/native.ts'

/**
 * The Rust rules for scripts that run domain code without the core services, resolved as the
 * core resolves them. Installs them as the domain's rules binding and returns the module.
 */
export function installNativeRules(): NativeRules {
  const native = createRequire(new URL('../core/src/services/native.ts', import.meta.url))(
    '@kchess/native',
  ) as NativeRules
  setRulesBinding({ invoke: (method, args) => native.invoke(method, args) })
  return native
}
