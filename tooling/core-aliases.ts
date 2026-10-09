import { fileURLToPath } from 'node:url'

const src = (path: string): string => fileURLToPath(new URL(`../core/src/${path}`, import.meta.url))

/**
 * The core's public specifiers, resolved to source for frontends that bundle the core
 * (the desktop shell, renderer and unit tests). Node frontends resolve the same names
 * through the package's built exports. Keep in step with the tsconfig `paths` entries.
 */
export const coreAliases = [
  { find: /^@kchess\/core\/(domain|contracts)\//, replacement: `${src('')}$1/` },
  { find: /^@kchess\/core\/logger$/, replacement: src('services/logger.ts') },
  { find: /^@kchess\/core$/, replacement: src('index.ts') },
]

/** The renderer's subset, as Nuxt prefix aliases (also emitted as tsconfig paths). */
export const rendererCoreAliases = {
  '@kchess/core/domain': src('domain'),
  '@kchess/core/contracts': src('contracts'),
}
