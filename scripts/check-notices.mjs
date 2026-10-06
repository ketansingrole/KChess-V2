import { readFileSync } from 'node:fs'

/**
 * Every runtime dependency ships in the app bundle, so each one must be
 * attributed in THIRD_PARTY_NOTICES.md. Dev-only tooling is excluded: it
 * never reaches users. Run via `pnpm run check:notices`.
 */
const notices = readFileSync(new URL('../THIRD_PARTY_NOTICES.md', import.meta.url), 'utf8')
const { dependencies = {} } = JSON.parse(
  readFileSync(new URL('../package.json', import.meta.url), 'utf8'),
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
