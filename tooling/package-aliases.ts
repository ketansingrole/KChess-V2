import { fileURLToPath } from 'node:url'

const dir = (path: string): string => fileURLToPath(new URL(`../${path}`, import.meta.url))

/** The workspace packages' source directories. */
export const packageDirs = {
  '@kchess/native': dir('crates/kchess-node/js'),
  '@kchess/rules': dir('crates/kchess-wasm/js'),
  '@kchess/contracts': dir('crates/kchess-contracts/ts'),
} as const

/**
 * The packages' specifiers, resolved to source for frontends that bundle them (the desktop shell,
 * renderer and unit tests). Node frontends resolve the same names through the packages' built
 * `exports`. Keep in step with `tsconfig.packages.json` and each package's `exports`.
 */
export const packageAliases = [
  { find: /^@kchess\/native$/, replacement: `${packageDirs['@kchess/native']}/index.ts` },
  { find: /^@kchess\/native\/(.+)$/, replacement: `${packageDirs['@kchess/native']}/$1` },
  { find: /^@kchess\/rules\/(.+)$/, replacement: `${packageDirs['@kchess/rules']}/$1` },
  { find: /^@kchess\/contracts\/(.+)$/, replacement: `${packageDirs['@kchess/contracts']}/$1` },
]

/** The renderer's subset, as Nuxt prefix aliases (also emitted as tsconfig paths). */
export const rendererPackageAliases = {
  '@kchess/rules': packageDirs['@kchess/rules'],
  '@kchess/contracts': packageDirs['@kchess/contracts'],
}
