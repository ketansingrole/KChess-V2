import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'
import { expect, it } from 'vitest'
import { mainPushes, pushProblem } from '../../tooling/pre-push.mjs'

const head = 'a'.repeat(40)
const other = 'b'.repeat(40)
const zero = '0'.repeat(40)

it('selects only pushes that update main', () => {
  const input = [
    `refs/heads/feature ${other} refs/heads/feature ${zero}`,
    `refs/heads/main ${head} refs/heads/main ${other}`,
    `refs/tags/v2026.10.1 ${head} refs/tags/v2026.10.1 ${zero}`,
    `(delete) ${zero} refs/heads/main ${other}`,
    '',
  ].join('\n')
  expect(mainPushes(input)).toEqual([{ localRef: 'refs/heads/main', localSha: head }])
  expect(mainPushes(`HEAD ${head} refs/heads/mainline ${zero}\n`)).toEqual([])
  expect(mainPushes('')).toEqual([])
})
it('refuses to vouch for a commit or tree that check would not test', () => {
  const pushes = [{ localRef: 'refs/heads/main', localSha: head }]
  expect(pushProblem(pushes, head, '')).toBeUndefined()
  expect(pushProblem(pushes, other, '')).toContain('not the checked-out HEAD')
  expect(pushProblem(pushes, head, ' M apps/desktop/app/pages/history.vue\n')).toContain(
    'uncommitted or untracked',
  )
  expect(pushProblem(pushes, head, '?? tests/unit/new.test.ts\n')).toContain(
    'tests/unit/new.test.ts',
  )
})
// Vitest runs from the repository root; happy-dom gives import.meta.url an http scheme.
const hook = resolve('tooling/pre-push.mjs')
function runHook(input, env = {}) {
  const result = spawnSync(process.execPath, [hook, 'origin'], {
    input,
    encoding: 'utf8',
    env: { ...process.env, KCHESS_SKIP_PRE_PUSH: '', ...env },
  })
  return { status: result.status, output: result.stdout + result.stderr }
}
it('lets other branches push without running check', () => {
  const result = runHook(`refs/heads/feature ${head} refs/heads/feature ${zero}\n`)
  expect(result).toEqual({ status: 0, output: '' })
})
it('refuses a main push of another commit and skips only on purpose', () => {
  // `other` is never this checkout's HEAD, so the hook refuses before running check.
  const push = `refs/heads/main ${other} refs/heads/main ${zero}\n`
  const refused = runHook(push)
  expect(refused.status).toBe(1)
  expect(refused.output).toContain('Push to main refused')
  expect(runHook(push, { KCHESS_SKIP_PRE_PUSH: 'true' }).status).toBe(1)
  const skipped = runHook(push, { KCHESS_SKIP_PRE_PUSH: '1' })
  expect(skipped.status).toBe(0)
  expect(skipped.output).toContain('without `pnpm run check`')
})
