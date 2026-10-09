import { builtinModules } from 'node:module'
import { resolve, relative } from 'node:path'

const builtins = new Set(builtinModules.map((name) => name.replace(/^node:/, '')))
const normalize = (path) => path.replaceAll('\\', '/')
/** A path inside `dir`, including the directory's own index. */
const within = (path, dir) => path === dir || path.startsWith(`${dir}/`)
/** What a frontend may import from the core: its entry and the shared logger. */
const PUBLIC_CORE = ['core/src', 'core/src/index', 'core/src/services/logger']
const SQLITE_OWNERS = ['core/src/services/db.ts', 'core/src/services/store.ts']

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
          : source === '@kchess/core'
            ? 'core/src'
            : source === '@kchess/core/logger'
              ? 'core/src/services/logger'
              : /^@kchess\/core\/(?:domain|contracts)\//.test(source)
                ? 'core/src/' + source.slice('@kchess/core/'.length)
                : source.startsWith('@kchess/core/')
                  ? 'core/src/domain/' + source.slice('@kchess/core/'.length)
                  : source === '@kchess/node'
                    ? 'hosts/node/src'
                    : source
      const renderer =
        file.startsWith('apps/desktop/app/') || file.startsWith('apps/desktop/electron/renderer/')
      const shared =
        file.startsWith('core/src/domain/') ||
        file.startsWith('core/src/contracts/') ||
        file.startsWith('apps/desktop/contracts/')
      const preload = file.startsWith('apps/desktop/electron/preload/')
      const main = file.startsWith('apps/desktop/electron/main/')
      const core = file.startsWith('core/src/services/')
      const nodeHost = file.startsWith('hosts/node/src/') || file.startsWith('apps/cli/src/')
      let reason
      if (
        file.startsWith('core/') &&
        (imported.startsWith('apps/') || imported.startsWith('hosts/'))
      )
        reason = 'The core cannot depend on an application or host.'
      else if (
        (renderer || shared || preload) &&
        (within(imported, 'apps/desktop/electron/main') ||
          (within(imported, 'core/src') &&
            !within(imported, 'core/src/domain') &&
            !within(imported, 'core/src/contracts')) ||
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
        within(imported, 'core/src/services') &&
        !PUBLIC_CORE.includes(imported.replace(/\.ts$/, ''))
      )
        reason = 'Hosts use only the core entry (core/src) and its logger.'
      else if (
        (main || preload || renderer || nodeHost || file.startsWith('apps/desktop/contracts/')) &&
        !source.startsWith('@kchess/') &&
        within(imported, 'core/src')
      )
        reason =
          'Import the core by its public names: @kchess/core, @kchess/core/logger, @kchess/core/domain/*, @kchess/core/contracts/*.'
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
        (file.startsWith('apps/desktop/electron/main/') || file.startsWith('core/src/services/')) &&
        !SQLITE_OWNERS.includes(file)
      )
        context.report({
          node,
          messageId: 'boundary',
          data: {
            reason: 'Open SQLite only in its database owner.',
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
        if (node.source.value === 'node:sqlite' && !SQLITE_OWNERS.includes(file)) {
          for (const specifier of node.specifiers) {
            if (node.importKind !== 'type' && specifier.importKind !== 'type')
              context.report({
                node: specifier,
                messageId: 'boundary',
                data: {
                  reason:
                    'Open SQLite only in its database owner (store retains the legacy migration).',
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
