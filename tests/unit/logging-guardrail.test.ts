import { describe, expect, it } from 'vitest'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { rules } from '../../scripts/eslint/logging.mjs'

// Regression for silent error swallowing: every catch and every inline
// `.catch()` must log or rethrow so production logs can explain any failure.
// `pnpm run lint` enforces this via the custom ESLint rules; this test keeps
// the guarantee even if lint is skipped, by scanning the shipped sources for
// the exact silent patterns that shipped before the guardrail.

function sources(dir: string): string[] {
  const out: string[] = []
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry)
    if (statSync(path).isDirectory()) {
      if (['node_modules', '.nuxt', '.output', 'out', 'dist'].includes(entry)) continue
      out.push(...sources(path))
    } else if (/\.(ts|vue)$/.test(entry)) out.push(path)
  }
  return out
}

describe('logging guardrails', () => {
  it('exports the no-silent-error rules', () => {
    expect(Object.keys(rules).sort()).toEqual([
      'no-raw-console',
      'no-silent-catch',
      'no-silent-promise-catch',
    ])
  })

  it('has no silent inline promise catches in shipped sources', () => {
    const roots = ['src/core', 'src/main', 'src/shared', 'src/preload', 'app']
    const silent: string[] = []
    for (const root of roots) {
      for (const file of sources(root)) {
        const text = readFileSync(file, 'utf8')
        // Matches `.catch(() => {})`, `.catch(() => null/undefined/[]/false/0)`
        // and `.catch((x) => x)`-style fallbacks without a log/throw body.
        const pattern =
          /\.catch\(\s*(?:\(\s*\)|\(\s*[_a-zA-Z][\w]*\s*\))\s*=>\s*(?:\{\s*\}|null|undefined|\[\]|false|0)\s*\)/g
        let match: RegExpExecArray | null
        while ((match = pattern.exec(text))) {
          // Allow the diagnostics pipeline's own recursion guard, which
          // carries an explicit eslint-disable justification.
          const before = text.slice(Math.max(0, match.index - 200), match.index)
          if (before.includes('eslint-disable-next-line logging/no-silent-promise-catch')) continue
          silent.push(`${file}: silent ${match[0].slice(0, 40)}`)
        }
      }
    }
    expect(silent, silent.join('\n')).toEqual([])
  })

  it('has no empty catch blocks in shipped sources', () => {
    const roots = ['src/core', 'src/main', 'src/shared', 'src/preload', 'app']
    const silent: string[] = []
    for (const root of roots) {
      for (const file of sources(root)) {
        const text = readFileSync(file, 'utf8')
        const pattern = /catch\s*(?:\([^)]*\))?\s*\{\s*\}/g
        let match: RegExpExecArray | null
        while ((match = pattern.exec(text))) {
          const before = text.slice(Math.max(0, match.index - 200), match.index)
          if (before.includes('eslint-disable-next-line logging/no-silent-catch')) continue
          // Block disables in diagnostics.ts carry their justification.
          if (before.includes('eslint-disable logging/no-silent-catch')) continue
          silent.push(`${file}: empty catch`)
        }
      }
    }
    expect(silent, silent.join('\n')).toEqual([])
  })
})
