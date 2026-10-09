import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { parse } from 'yaml'

const VERSION = /^(20[0-9]{2})\.([1-9]|1[0-2])\.(0|[1-9][0-9]*)$/

export function parseReleaseVersion(version) {
  const match = typeof version === 'string' && version.match(VERSION)
  if (!match || match[0] !== version || !Number.isSafeInteger(Number(match[3])))
    throw new Error(
      `Invalid release version ${version}: use YEAR.MONTH.COUNTER, without zero padding (e.g. 2026.10.0).`,
    )
  return { year: Number(match[1]), month: Number(match[2]), counter: Number(match[3]) }
}

export function nextReleaseVersion(tags, now = new Date()) {
  const year = now.getUTCFullYear()
  const month = now.getUTCMonth() + 1
  let counter = -1
  for (const tag of tags) {
    if (!tag.startsWith('v') || !VERSION.test(tag.slice(1))) continue
    const parsed = parseReleaseVersion(tag.slice(1))
    if (parsed.year > year || (parsed.year === year && parsed.month > month))
      throw new Error(
        `Tag ${tag} is ahead of the current UTC month. Check the clock and tags before releasing.`,
      )
    if (parsed.year === year && parsed.month === month) counter = Math.max(counter, parsed.counter)
  }
  const version = `${year}.${month}.${counter + 1}`
  parseReleaseVersion(version)
  return version
}

export function validateReleaseVersion(pkg, lock, tag, importerName = '.', desktop = pkg) {
  parseReleaseVersion(pkg.version)
  const importer = lock.importers?.[importerName]
  if (!importer || String(lock.lockfileVersion) !== '9.0')
    throw new Error('Expected a pnpm lockfile with a root importer.')
  for (const group of ['dependencies', 'devDependencies', 'optionalDependencies']) {
    const declared = pkg[group] ?? {}
    const locked = importer[group] ?? {}
    if (
      Object.keys(declared).length !== Object.keys(locked).length ||
      Object.entries(declared).some(
        ([name, specifier]) => locked[name]?.specifier !== specifier || !locked[name]?.version,
      )
    )
      throw new Error('package.json dependencies and pnpm-lock.yaml must match. Run pnpm install.')
  }
  if (tag !== undefined && tag !== `v${pkg.version}`)
    throw new Error(`Tag ${tag} must be exactly v${pkg.version}.`)
  for (const target of desktop.build.mac.target)
    if (target.arch?.length !== 1 || target.arch[0] !== 'arm64')
      throw new Error('macOS releases must target Apple Silicon (arm64) only.')
  return `v${pkg.version}`
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = new URL('..', import.meta.url)
  const pkg = JSON.parse(readFileSync(new URL('package.json', root), 'utf8'))
  const lock = parse(readFileSync(new URL('pnpm-lock.yaml', root), 'utf8'))
  const tag = process.env.GITHUB_REF?.startsWith('refs/tags/')
    ? process.env.GITHUB_REF_NAME
    : undefined
  const desktop = JSON.parse(readFileSync(new URL('apps/desktop/package.json', root), 'utf8'))
  for (const name of ['.', 'core', 'hosts/node', 'apps/cli', 'apps/desktop']) {
    const workspace = JSON.parse(readFileSync(new URL(name + '/package.json', root), 'utf8'))
    if (workspace.version !== pkg.version) throw new Error('Workspace release versions must match.')
    validateReleaseVersion(workspace, lock, tag, name, desktop)
  }
  console.info(`Release metadata verified: v${pkg.version}`)
}
