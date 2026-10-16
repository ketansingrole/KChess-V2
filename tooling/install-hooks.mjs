// Points Git at the repository's versioned hooks (.githooks). Runs from `pnpm install`
// (`prepare`). It leaves a different hooks path the developer chose on purpose alone.
import { spawnSync } from 'node:child_process'

function git(...args) {
  const result = spawnSync('git', args, { encoding: 'utf8' })
  return { ok: result.status === 0, out: (result.stdout ?? '').trim() }
}

// Not a Git checkout (e.g. an unpacked source archive): nothing to install.
if (git('rev-parse', '--is-inside-work-tree').out !== 'true') process.exit(0)
const current = git('config', '--get', 'core.hooksPath').out
if (current === '.githooks') process.exit(0)
if (current) {
  console.warn(`[kchess] core.hooksPath is ${current}; not installing .githooks (pre-push check).`)
  process.exit(0)
}
if (git('config', 'core.hooksPath', '.githooks').ok)
  console.info('[kchess] Installed Git hooks from .githooks (pre-push check for main).')
else console.warn('[kchess] Could not set core.hooksPath to .githooks.')
