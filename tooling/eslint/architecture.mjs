import { builtinModules } from 'node:module'
import { resolve, relative } from 'node:path'

const builtins = new Set(builtinModules.map((name) => name.replace(/^node:/, '')))
const normalize = (path) => path.replaceAll('\\', '/')
/** A path inside `dir`, including the directory's own index. */
const within = (path, dir) => path === dir || path.startsWith(`${dir}/`)

/** The headless core's packages: the Node glue and services, the rules, and the contracts. */
const NATIVE = 'crates/kchess-node/js'
const RULES = 'crates/kchess-wasm/js'
const CONTRACTS = 'crates/kchess-contracts/ts'
const PACKAGES = [
  ['@kchess/native', NATIVE],
  ['@kchess/rules', RULES],
  ['@kchess/contracts', CONTRACTS],
]
/** What a frontend may import from the core: the native entry and the shared logger. */
const PUBLIC_CORE = [NATIVE, `${NATIVE}/index`, `${NATIVE}/logger`]
/** Where the core and Electron main never open SQLite, and their tests. */
const SQLITE_FREE = [NATIVE, RULES, CONTRACTS, 'tests/core']

const inCore = (path) => [NATIVE, RULES, CONTRACTS].some((dir) => within(path, dir))

/** The repository path a workspace package specifier names, or undefined for other specifiers. */
function packagePath(source) {
  for (const [name, dir] of PACKAGES) {
    if (source === name) return dir
    if (source.startsWith(`${name}/`)) return `${dir}/${source.slice(name.length + 1)}`
  }
  return undefined
}

export const boundaries = {
  meta: {
    type: 'problem',
    schema: [],
    messages: { boundary: '{{reason}}' },
  },
  create(context) {
    const file = normalize(relative(context.cwd, context.filename))
    function check(node, source) {
      if (typeof source !== 'string') return
      const imported = source.startsWith('.')
        ? normalize(relative(context.cwd, resolve(context.filename, '..', source)))
        : /^[@~]{1,2}\//.test(source)
          ? normalize(
              relative(
                context.cwd,
                resolve(
                  context.cwd,
                  /^[@~]\//.test(source) ? 'apps/desktop/app' : 'apps/desktop',
                  source.replace(/^[@~]{1,2}\//, ''),
                ),
              ),
            )
          : (packagePath(source) ?? (source === '@kchess/node' ? 'hosts/node/src' : source))
      const renderer =
        file.startsWith('apps/desktop/app/') || file.startsWith('apps/desktop/electron/renderer/')
      const shared =
        within(file, RULES) || within(file, CONTRACTS) || file.startsWith('apps/desktop/contracts/')
      const preload = file.startsWith('apps/desktop/electron/preload/')
      const main = file.startsWith('apps/desktop/electron/main/')
      const core = within(file, NATIVE)
      const nodeHost = file.startsWith('hosts/node/src/') || file.startsWith('apps/cli/src/')
      let reason
      if (inCore(file) && (imported.startsWith('apps/') || imported.startsWith('hosts/')))
        reason = 'The core cannot depend on an application or host.'
      else if (
        (renderer || shared || preload) &&
        (within(imported, 'apps/desktop/electron/main') ||
          within(imported, NATIVE) ||
          within(imported, 'hosts/node/src') ||
          within(imported, 'apps/cli/src'))
      )
        reason = 'Main-process services are accessible only through DesktopApi.'
      else if (
        (main || core || shared || preload || nodeHost) &&
        imported.startsWith('apps/desktop/app/')
      )
        reason = 'Main, core, preload and shared code cannot depend on renderer code.'
      else if (
        core &&
        (/^electron(?:-|\/|$)/.test(source) ||
          within(imported, 'apps/desktop/electron/main') ||
          within(imported, 'apps/desktop/electron/preload') ||
          within(imported, 'hosts/node/src') ||
          within(imported, 'apps/cli/src') ||
          /^(?:vue|pinia|nuxt)(?:$|\/)|^@(?:nuxt|vueuse)\//.test(source))
      )
        reason =
          'The headless core cannot depend on Electron or the desktop shell; add a CorePlatform capability.'
      else if (shared && /^(?:vue|pinia|nuxt)(?:$|\/)|^@(?:nuxt|vueuse)\//.test(source))
        reason = 'Shared rules and controllers cannot depend on a frontend framework.'
      else if (
        (renderer || shared) &&
        (source === 'electron' || builtins.has(source.replace(/^node:/, '')))
      )
        reason = 'Renderer and shared code cannot import privileged Electron or Node APIs.'
      else if (
        (main || nodeHost) &&
        within(imported, NATIVE) &&
        !PUBLIC_CORE.includes(imported.replace(/\.ts$/, ''))
      )
        reason = 'Hosts use only the native entry (@kchess/native) and its logger.'
      else if (
        (main || preload || renderer || nodeHost || file.startsWith('apps/desktop/contracts/')) &&
        !source.startsWith('@kchess/') &&
        inCore(imported)
      )
        reason =
          'Import the core by its public names: @kchess/native, @kchess/native/logger, @kchess/rules/*, @kchess/contracts/*.'
      if (reason) context.report({ node, messageId: 'boundary', data: { reason } })
    }
    function privilegedLoad(node, source) {
      if (
        source === 'electron' &&
        file.startsWith('apps/desktop/electron/main/') &&
        file !== 'apps/desktop/electron/main/ipc.ts'
      )
        context.report({
          node,
          messageId: 'boundary',
          data: {
            reason: 'Use named Electron imports so IPC ownership can be checked.',
          },
        })
      if (
        source === 'node:sqlite' &&
        (file.startsWith('apps/desktop/electron/main/') ||
          SQLITE_FREE.some((dir) => within(file, dir)))
      )
        context.report({
          node,
          messageId: 'boundary',
          data: {
            reason: 'The core and Electron main never open SQLite; the Rust core owns kchess.db.',
          },
        })
    }
    return {
      ImportDeclaration(node) {
        check(node, node.source.value)
        if (node.source.value === 'electron' && file !== 'apps/desktop/electron/main/ipc.ts') {
          for (const specifier of node.specifiers) {
            if (
              specifier.imported?.name === 'ipcMain' ||
              specifier.type === 'ImportNamespaceSpecifier' ||
              specifier.type === 'ImportDefaultSpecifier'
            )
              context.report({
                node: specifier,
                messageId: 'boundary',
                data: { reason: 'Register IPC only through the authenticated handle wrapper.' },
              })
          }
        }
        if (
          node.source.value === 'node:sqlite' &&
          (file.startsWith('apps/desktop/electron/main/') ||
            SQLITE_FREE.some((dir) => within(file, dir)))
        ) {
          for (const specifier of node.specifiers) {
            if (node.importKind !== 'type' && specifier.importKind !== 'type')
              context.report({
                node: specifier,
                messageId: 'boundary',
                data: {
                  reason:
                    'The core and Electron main never open SQLite; the Rust core owns kchess.db.',
                },
              })
          }
        }
      },
      ExportNamedDeclaration(node) {
        if (node.source) check(node, node.source.value)
      },
      ExportAllDeclaration(node) {
        check(node, node.source.value)
      },
      ImportExpression(node) {
        check(node, node.source.value)
        privilegedLoad(node, node.source.value)
      },
      CallExpression(node) {
        if (node.callee.name === 'require') {
          check(node, node.arguments[0]?.value)
          privilegedLoad(node, node.arguments[0]?.value)
        }
      },
    }
  },
}

export default { rules: { boundaries } }
