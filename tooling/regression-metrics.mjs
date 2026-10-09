import { execFileSync } from 'node:child_process'

/** Count triaged released regressions; an absent classification is unavailable, never zero. */
export function regressionMetrics(repository, execute = execFileSync, now = Date.now()) {
  const since = new Date(now - 30 * 86_400_000).toISOString().slice(0, 10)
  const labels = JSON.parse(
    execute('gh', ['label', 'list', '--repo', repository, '--limit', '1000', '--json', 'name'], {
      encoding: 'utf8',
    }),
  )
  if (!labels.some((label) => label.name === 'regression'))
    return { since, count: null, reason: 'The regression classification label is unavailable.' }
  const issues = JSON.parse(
    execute(
      'gh',
      [
        'issue',
        'list',
        '--repo',
        repository,
        '--state',
        'all',
        '--label',
        'regression',
        '--search',
        'created:>=' + since,
        '--limit',
        '1000',
        '--json',
        'number,createdAt',
      ],
      { encoding: 'utf8' },
    ),
  )
  return { since, count: issues.length, truncated: issues.length === 1000 }
}
