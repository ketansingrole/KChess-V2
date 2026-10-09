import { readFileSync } from 'node:fs'

/**
 * Every runtime dependency ships in the app bundle, so each one must be
 * attributed in THIRD_PARTY_NOTICES.md. Dev-only tooling is excluded: it
 * never reaches users. Run via `pnpm run check:notices`.
 */
const notices = readFileSync(new URL('../THIRD_PARTY_NOTICES.md', import.meta.url), 'utf8')
const dependencies = Object.assign(
  {},
  ...['core', 'hosts/node', 'apps/cli', 'apps/desktop'].map((path) => {
    const pkg = JSON.parse(
      readFileSync(new URL('../' + path + '/package.json', import.meta.url), 'utf8'),
    )
    return Object.fromEntries(
      Object.entries(pkg.dependencies ?? {}).filter(
        ([, version]) => !version.startsWith('workspace:'),
      ),
    )
  }),
)

const escape = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
const missing = Object.keys(dependencies).filter(
  (name) => !new RegExp(`(?<![\\w@/-])${escape(name)}(?![\\w-])`, 'i').test(notices),
)

if (missing.length) {
  console.error(`THIRD_PARTY_NOTICES.md is missing entries for: ${missing.join(', ')}`)
  console.error('Add each shipped dependency with its license and what it is used for.')
  process.exit(1)
}
console.info(
  `[kchess] All ${Object.keys(dependencies).length} runtime dependencies are attributed.`,
)
