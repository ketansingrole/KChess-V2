/**
 * Logging guardrails: errors and warnings must never be swallowed silently.
 *
 * - `logging/no-silent-catch`: every `catch` block must log (logInfo/logWarn/
 *   logError/logDebug or console.*) or rethrow. Empty blocks and pure
 *   fallbacks (`return false`, `return null`) fail.
 * - `logging/no-silent-promise-catch`: every inline `.catch(fn)` must log or
 *   rethrow. Non-inline handlers (identifiers, `noSuchUser(x)`) are assumed to
 *   be named handlers that throw or log at their definition.
 * - `logging/no-raw-console`: main-process code must go through the scoped
 *   `crates/kchess-node/js/logger.ts` so entries carry `[scope]` and pass through redacted
 *   `DiagnosticLog`. Only `logger.ts` and `diagnostics.ts` may touch console.
 */

const LOG_FNS = new Set(['logInfo', 'logWarn', 'logError', 'logDebug'])
const CONSOLE_METHODS = new Set(['log', 'info', 'warn', 'error', 'debug'])

function isLoggingCall(node) {
  if (node?.type !== 'CallExpression') return false
  const callee = node.callee
  if (callee.type === 'Identifier' && LOG_FNS.has(callee.name)) return true
  if (
    callee.type === 'MemberExpression' &&
    !callee.computed &&
    callee.object.type === 'Identifier' &&
    callee.object.name === 'console' &&
    callee.property.type === 'Identifier' &&
    CONSOLE_METHODS.has(callee.property.name)
  )
    return true
  return false
}

function containsThrow(node) {
  let found = false
  const visit = (current) => {
    if (!current || found) return
    if (Array.isArray(current)) {
      for (const child of current) visit(child)
      return
    }
    if (typeof current !== 'object' || !current.type) return
    // Do not count logging inside a nested function as handling the outer error.
    if (
      current !== node &&
      (current.type === 'FunctionExpression' ||
        current.type === 'ArrowFunctionExpression' ||
        current.type === 'FunctionDeclaration')
    )
      return
    if (current.type === 'ThrowStatement') {
      found = true
      return
    }
    for (const key of Object.keys(current)) {
      if (key === 'parent') continue
      visit(current[key])
    }
  }
  visit(node)
  return found
}

function containsLogging(node) {
  let found = false
  const visit = (current) => {
    if (!current || found) return
    if (Array.isArray(current)) {
      for (const child of current) visit(child)
      return
    }
    if (typeof current !== 'object' || !current.type) return
    if (
      current !== node &&
      (current.type === 'FunctionExpression' ||
        current.type === 'ArrowFunctionExpression' ||
        current.type === 'FunctionDeclaration')
    )
      return
    if (isLoggingCall(current)) {
      found = true
      return
    }
    for (const key of Object.keys(current)) {
      if (key === 'parent') continue
      visit(current[key])
    }
  }
  visit(node)
  return found
}

const noSilentCatch = {
  meta: {
    type: 'problem',
    docs: {
      description: 'Require every catch block to log or rethrow; never swallow errors silently.',
    },
    messages: {
      silent:
        'Silent catch: log with logWarn/logError/logDebug (main) or console.warn/error (renderer), or rethrow. Swallowed errors are undebuggable in production logs.',
    },
  },
  create(context) {
    return {
      CatchClause(node) {
        if (containsLogging(node.body) || containsThrow(node.body)) return
        context.report({ node, messageId: 'silent' })
      },
    }
  },
}

const noSilentPromiseCatch = {
  meta: {
    type: 'problem',
    docs: {
      description: 'Require every inline .catch() handler to log or rethrow.',
    },
    messages: {
      silent:
        'Silent .catch(): log the failure (logWarn/logError/logDebug with scope, or console.warn/error in renderer) or rethrow. Use `.catch((error) => { logWarn(...); return fallback })` for expected fallbacks.',
    },
  },
  create(context) {
    return {
      CallExpression(node) {
        const callee = node.callee
        if (callee.type !== 'MemberExpression' || callee.computed) return
        if (callee.property.type !== 'Identifier' || callee.property.name !== 'catch') return
        const handler = node.arguments[0]
        if (!handler) {
          context.report({ node, messageId: 'silent' })
          return
        }
        // Named handlers (`noSuchUser(x)`, `fail`, identifiers) are defined
        // elsewhere; the throw/log requirement applies at their definition.
        if (handler.type !== 'ArrowFunctionExpression' && handler.type !== 'FunctionExpression')
          return
        const body = handler.body
        if (containsLogging(body) || containsThrow(body)) return
        // `() => fallback` without logging swallows the reason.
        context.report({ node: handler, messageId: 'silent' })
      },
    }
  },
}

const noRawConsole = {
  meta: {
    type: 'problem',
    docs: {
      description: 'Main-process code must log through the scoped logger, not raw console.',
    },
    messages: {
      raw: 'Use logInfo/logWarn/logError/logDebug with a scope instead of raw console.* so entries are greppable and redacted.',
    },
  },
  create(context) {
    const file = context.filename.replaceAll('\\', '/')
    const allowed =
      file.endsWith('crates/kchess-node/js/logger.ts') ||
      file.endsWith('apps/desktop/electron/main/diagnostics.ts')
    if (allowed) return {}
    if (!file.includes('apps/desktop/electron/main/') && !file.includes('crates/kchess-node/js/'))
      return {}
    return {
      MemberExpression(node) {
        if (
          !node.computed &&
          node.object.type === 'Identifier' &&
          node.object.name === 'console' &&
          node.property.type === 'Identifier' &&
          CONSOLE_METHODS.has(node.property.name) &&
          node.parent.type === 'CallExpression' &&
          node.parent.callee === node
        ) {
          context.report({ node: node.parent, messageId: 'raw' })
        }
      },
    }
  },
}

export const rules = {
  'no-silent-catch': noSilentCatch,
  'no-silent-promise-catch': noSilentPromiseCatch,
  'no-raw-console': noRawConsole,
}

export default { rules }
