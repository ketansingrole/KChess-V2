// Compatibility entry point. Lifecycle cases now compile and mount real Vue components in Vitest.
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
const result = spawnSync(
  process.execPath,
  [
    fileURLToPath(new URL('../node_modules/vitest/vitest.mjs', import.meta.url)),
    'run',
    'apps/desktop/tests/unit/lifecycle.test.ts',
    'core/tests/unit/online-session.test.ts',
  ],
  { stdio: 'inherit' },
)
if (result.error) console.error(result.error)
process.exitCode = result.status ?? 1
