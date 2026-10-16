// Git pre-push hook: pushes to main must pass the full `pnpm run check` first.
// Other branches push immediately; CI still runs everything on every push and pull request.
// Skip deliberately with KCHESS_SKIP_PRE_PUSH=1 (or git's own --no-verify).
import { execFileSync, spawn } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const MAIN = 'refs/heads/main'
const DELETED = /^0+$/

/** The local commits that the push sends to main, from the hook's stdin lines. */
export function mainPushes(input) {
  return input
    .split('\n')
    .map((line) => line.trim().split(/\s+/))
    .filter(([, localSha, remoteRef]) => remoteRef === MAIN && localSha && !DELETED.test(localSha))
    .map(([localRef, localSha]) => ({ localRef, localSha }))
}

/**
 * Why `check` cannot vouch for this push, or undefined when it can: it runs on the working
 * tree, so the pushed commit must be HEAD and nothing may be uncommitted or untracked.
 */
export function pushProblem(pushes, head, status) {
  const other = pushes.find(({ localSha }) => localSha !== head)
  if (other)
    return `${other.localRef} (${other.localSha.slice(0, 12)}) is not the checked-out HEAD; check it out to verify it.`
  if (status.trim())
    return (
      'the working tree has uncommitted or untracked files, which `pnpm run check` would include:\n' +
      status.trimEnd()
    )
  return undefined
}

function git(...args) {
  return execFileSync('git', args, { encoding: 'utf8' })
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const pushes = mainPushes(readFileSync(0, 'utf8'))
  if (!pushes.length) process.exit(0)
  if (process.env.KCHESS_SKIP_PRE_PUSH === '1') {
    console.warn('[kchess] KCHESS_SKIP_PRE_PUSH=1: pushing to main without `pnpm run check`.')
    process.exit(0)
  }
  const problem = pushProblem(
    pushes,
    git('rev-parse', 'HEAD').trim(),
    git('status', '--porcelain', '--untracked-files=normal'),
  )
  if (problem) {
    console.error(`[kchess] Push to main refused: ${problem}`)
    console.error('[kchess] Skip on purpose with KCHESS_SKIP_PRE_PUSH=1 git push …')
    process.exit(1)
  }
  console.info('[kchess] Pushing to main: running `pnpm run check`…')
  const child = spawn('pnpm', ['run', 'check'], {
    stdio: 'inherit',
    shell: process.platform === 'win32',
  })
  child.once('error', (error) => {
    console.error('[kchess] Could not run pnpm:', error)
    process.exit(1)
  })
  child.once('exit', (code) => {
    if (code) console.error('[kchess] Push to main refused: `pnpm run check` failed.')
    process.exit(code ?? 1)
  })
}
