import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

// Releases are versioned YEAR.MONTH.COUNTER (2026.10.0, 2026.10.1, … 2026.11.0): the counter
// restarts each month. No zero padding, so the version stays valid semver for npm and electron-builder.
const root = fileURLToPath(new URL('..', import.meta.url))
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim()

try {
  git('fetch', '--tags', '--quiet')
} catch {
  console.warn('Could not fetch tags; using the tags already in this checkout.')
}

const now = new Date()
const month = `${now.getUTCFullYear()}.${now.getUTCMonth() + 1}`
const counters = git('tag', '--list', `v${month}.*`)
  .split('\n')
  .map((tag) => tag.match(/^v\d+\.\d+\.(0|[1-9]\d*)$/)?.[1])
  .filter((counter) => counter !== undefined)
  .map(Number)
const version = `${month}.${counters.length ? Math.max(...counters) + 1 : 0}`

execFileSync('npm', ['version', version, '--no-git-tag-version'], {
  cwd: root,
  stdio: 'ignore',
  shell: process.platform === 'win32',
})
console.log(`package.json is now ${version}. Commit it, then tag and push:

  git commit -am "Release ${version}"
  git tag v${version}
  git push origin HEAD v${version}`)
