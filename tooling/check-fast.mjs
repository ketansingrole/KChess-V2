import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { runChecks } from './run-checks.mjs'

export function selectChecks(files) {
  const relevant = files.filter((file) => !file.startsWith('docs/'))
  const full = relevant.some((file) =>
    /^(?:(?:.*\/)?package\.json|core\/src\/index\.ts|pnpm-(?:lock|workspace)\.yaml|\.npmrc|.*config\.[^/]+|\.github\/|tooling\/|core\/src\/contracts\/|apps\/desktop\/contracts\/|core\/src\/domain\/|core\/src\/services\/(?:index|platform|service)\.ts$|AGENTS\.md)/.test(
      file,
    ),
  )
  const code = relevant.filter((file) => /\.(?:[cm]?[jt]s|vue)$/.test(file))
  const tests = code.filter((file) => /^(?:.*\/)?tests\/unit\/.*\.test\./.test(file))
  const sources = code.filter((file) => /^(?:apps|core|hosts)\//.test(file))
  const steps = []
  if (relevant.length)
    steps.push(['format', ['exec', 'prettier', '--check', '--ignore-unknown', ...relevant]])
  if (code.length)
    steps.push(['lint', ['exec', 'eslint', '--no-warn-ignored', '--max-warnings', '0', ...code]])
  if (full) {
    steps.push(['types', ['run', 'typecheck']], ['unit', ['run', 'test:unit']])
  } else {
    if (tests.length) steps.push(['unit-changed', ['exec', 'vitest', 'run', ...tests]])
    if (sources.length)
      steps.push([
        'unit-related',
        ['exec', 'vitest', 'related', '--run', '--passWithNoTests', ...sources],
      ])
  }
  return steps
}

export function changedFiles(base) {
  const revision = execFileSync(
    'git',
    ['rev-parse', '--verify', '--end-of-options', (base ?? 'HEAD') + '^{commit}'],
    { encoding: 'utf8' },
  ).trim()
  const tracked = execFileSync(
    'git',
    ['diff', '--name-only', '--diff-filter=ACMR', '-z', revision],
    { encoding: 'utf8' },
  )
  const untracked = execFileSync('git', ['ls-files', '--others', '--exclude-standard', '-z'], {
    encoding: 'utf8',
  })
  const deleted = execFileSync('git', ['diff', '--name-only', '--diff-filter=D', '-z', revision], {
    encoding: 'utf8',
  })
  // Removed code has no file to lint or trace through Vitest's related graph.
  const existing = (tracked + untracked).split('\0').filter((file) => file && existsSync(file))
  return [...new Set(deleted ? ['package.json', ...existing] : existing)]
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2)
  if (args.length && (args.length !== 2 || args[0] !== '--base'))
    throw new Error('Usage: pnpm run check:fast [--base <git-ref>]')
  const steps = selectChecks(changedFiles(args[1]))
  if (!steps.length) console.info('[kchess] No changed files to check.')
  process.exitCode = await runChecks(steps, 'fast')
}
