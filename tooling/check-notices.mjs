import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'

/**
 * Every runtime dependency ships in the app bundle, so each one must be
 * attributed in THIRD_PARTY_NOTICES.md. Dev-only tooling is excluded: it
 * never reaches users. Run via `pnpm run check:notices`.
 */
const notices = readFileSync(new URL('../THIRD_PARTY_NOTICES.md', import.meta.url), 'utf8')
const dependencies = Object.assign(
  {},
  ...[
    'crates/kchess-contracts/ts',
    'crates/kchess-wasm/js',
    'hosts/node',
    'apps/cli',
    'apps/desktop',
  ].map((path) => {
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

// The renderer is bundled by Nuxt from the desktop's devDependencies; these reach users.
const RENDERER_BUNDLED = [
  '@iconify-json/lucide',
  '@lichess-org/chessground',
  '@nuxt/ui',
  '@vueuse/core',
  'electron',
  'nuxt',
  'pinia',
  'tailwindcss',
  'vue',
  'vue-router',
]
const desktop = JSON.parse(
  readFileSync(new URL('../apps/desktop/package.json', import.meta.url), 'utf8'),
)
for (const name of RENDERER_BUNDLED) {
  if (!desktop.devDependencies?.[name] && !desktop.dependencies?.[name])
    throw new Error(`${name} is no longer a desktop dependency; update RENDERER_BUNDLED.`)
  dependencies[name] = 'renderer'
}

// Rust crates the shipped native module depends on directly (path crates are KChess's own),
// read with Cargo itself: a hand-written TOML scan skipped entries after inline arrays.
const SHIPPED_CRATES = ['kchess-core', 'kchess-domain', 'kchess-node']
const metadata = JSON.parse(
  execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps', '--offline'], {
    cwd: new URL('..', import.meta.url),
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  }),
)
for (const crate of SHIPPED_CRATES) {
  const pkg = metadata.packages.find((candidate) => candidate.name === crate)
  if (!pkg) throw new Error(`Cargo metadata has no ${crate} package.`)
  for (const dependency of pkg.dependencies)
    if (dependency.kind === null && !dependency.path) dependencies[dependency.name] = 'cargo'
}

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
