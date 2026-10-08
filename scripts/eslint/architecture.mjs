import { builtinModules } from 'node:module'
import { resolve, relative } from 'node:path'

const builtins = new Set(builtinModules.map((name) => name.replace(/^node:/, '')))
const normalize = (path) => path.replaceAll('\\', '/')
/** A path inside `dir`, including the directory's own index. */
const within = (path, dir) => path === dir || path.startsWith(`${dir}/`)
/** What a frontend may import from the core: its entry and the shared logger. */
const PUBLIC_CORE = ['src/core', 'src/core/index', 'src/core/logger']
const SQLITE_OWNERS = ['src/core/db.ts', 'src/core/puzzleWorker.ts', 'src/core/store.ts']

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
                  /^[@~]\//.test(source) ? 'app' : '.',
                  source.replace(/^[@~]{1,2}\//, ''),
                ),
              ),
            )
          : source
      const renderer = file.startsWith('app/') || file.startsWith('src/renderer/')
      const shared = file.startsWith('src/shared/')
      const preload = file.startsWith('src/preload/')
      const main = file.startsWith('src/main/')
      const core = file.startsWith('src/core/')
      let reason
      if (
        (renderer || shared || preload) &&
        (within(imported, 'src/main') || within(imported, 'src/core'))
      )
        reason = 'Main-process services are accessible only through DesktopApi.'
      else if ((main || core || shared || preload) && imported.startsWith('app/'))
        reason = 'Main, core, preload and shared code cannot depend on renderer code.'
      else if (
        core &&
        (/^electron(?:-|\/|$)/.test(source) ||
          within(imported, 'src/main') ||
          within(imported, 'src/preload'))
      )
        reason =
          'The headless core cannot depend on Electron or the desktop shell; add a CorePlatform capability.'
      else if (
        (renderer || shared) &&
        (source === 'electron' || builtins.has(source.replace(/^node:/, '')))
      )
        reason = 'Renderer and shared code cannot import privileged Electron or Node APIs.'
      else if (
        main &&
        within(imported, 'src/core') &&
        !PUBLIC_CORE.includes(imported.replace(/\.ts$/, ''))
      )
        reason = 'The desktop shell uses only the core entry (src/core) and its logger.'
      else if (
        (main || core) &&
        /(?:^|\/)puzzle(?:Queries|Worker)(?:\.ts)?$/.test(imported) &&
        file !== 'src/core/puzzleWorker.ts'
      )
        reason = 'Only puzzleWorker owns puzzle queries; use the asynchronous puzzleDb service.'
      if (reason) context.report({ node, messageId: 'boundary', data: { reason } })
    }
    function privilegedLoad(node, source) {
      if (source === 'electron' && file.startsWith('src/main/') && file !== 'src/main/ipc.ts')
        context.report({
          node,
          messageId: 'boundary',
          data: {
            reason: 'Use named Electron imports so IPC ownership can be checked.',
          },
        })
      if (
        source === 'node:sqlite' &&
        (file.startsWith('src/main/') || file.startsWith('src/core/')) &&
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
        if (node.source.value === 'electron' && file !== 'src/main/ipc.ts') {
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
