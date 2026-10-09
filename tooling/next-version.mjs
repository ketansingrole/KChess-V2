import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { nextReleaseVersion } from './release-rules.mjs'

// Releases are versioned YEAR.MONTH.COUNTER (2026.10.0, 2026.10.1, … 2026.11.0): the counter
// restarts each month. No zero padding, so the version stays valid semver for npm and electron-builder.
const root = dirname(dirname(fileURLToPath(import.meta.url)))
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim()

// pnpm's lockfile records dependencies, not the root package version.
export function writeVersion(version, directory = root) {
  for (const packagePath of [
    'package.json',
    'core/package.json',
    'hosts/node/package.json',
    'apps/cli/package.json',
    'apps/desktop/package.json',
    'crates/kchess-node/package.json',
  ]) {
    const path = join(directory, packagePath)
    if (!existsSync(path) && packagePath !== 'package.json') continue
    const pkg = JSON.parse(readFileSync(path, 'utf8'))
    pkg.version = version
    writeFileSync(path, JSON.stringify(pkg, null, 2) + '\n')
  }
}

export function prepareNextVersion(runGit = git, write = writeVersion, now = new Date()) {
  try {
    runGit('fetch', 'origin', '--tags', '--quiet')
  } catch (cause) {
    throw new Error(
      'Could not fetch release tags from origin. Version was not changed; restore access and retry.',
      { cause },
    )
  }
  const version = nextReleaseVersion(runGit('tag', '--list').split('\n'), now)
  write(version)
  return version
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const version = prepareNextVersion()
  console.log(`package.json is now ${version}.
Review and explicitly stage every intended file, including new files. Run pnpm run check,
then commit the release change to main (directly or via a pull request). Follow AGENTS.md's release checklist.
The tag must be v${version}, pointing at that commit on origin/main with green CI.
Pushing the tag creates a draft release. Publishing is a separate action.`)
}
