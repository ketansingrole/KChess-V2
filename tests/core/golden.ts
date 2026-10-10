import { createHash } from 'node:crypto'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterAll, expect } from 'vitest'

/**
 * Golden outputs for rules that moved to Rust: hashes of what the TypeScript implementation
 * returned for each case before it was removed (see RUST_MIGRATION.md). Each suite file
 * (`golden/<file>.json`) belongs to one test file. `KCHESS_WRITE_GOLDEN=1` records the
 * `reference` (the TypeScript output while it still exists) or else the actual output; do that
 * only while porting or for an intended behaviour change.
 */
const WRITE = process.env.KCHESS_WRITE_GOLDEN === '1'

/** Sorted keys; fractions to 9 significant digits (V8's Math.exp differs in the last bit by CPU). */
export const canonical = (value: unknown): string =>
  JSON.stringify(value, (_key, inner: unknown) =>
    typeof inner === 'number' && !Number.isInteger(inner)
      ? Number(inner.toPrecision(9))
      : inner && typeof inner === 'object' && !Array.isArray(inner)
        ? Object.fromEntries(Object.entries(inner).sort(([a], [b]) => a.localeCompare(b)))
        : inner,
  ) ?? 'undefined'

const digest = (value: unknown): string =>
  createHash('sha256').update(canonical(value)).digest('hex').slice(0, 16)

/** A golden file: `check(suite, index, actual, reference?)` compares or records one case. */
export function goldenFile(name: string): {
  check(suite: string, index: number, actual: unknown, reference?: unknown): void
} {
  const path = join(dirname(fileURLToPath(import.meta.url)), 'golden', `${name}.json`)
  const saved: Record<string, string[]> = existsSync(path)
    ? JSON.parse(readFileSync(path, 'utf8'))
    : {}
  const recorded: Record<string, string[]> = {}
  afterAll(() => {
    if (WRITE && Object.keys(recorded).length)
      writeFileSync(path, JSON.stringify({ ...saved, ...recorded }, null, 1) + '\n')
  })
  return {
    check(suite, index, actual, reference) {
      if (WRITE) {
        ;(recorded[suite] ??= [])[index] = digest(reference === undefined ? actual : reference)
        return
      }
      const expected = saved[suite]?.[index]
      expect(expected, `golden ${name}/${suite} #${index} is recorded`).toBeDefined()
      expect(
        digest(actual),
        `golden ${name}/${suite} #${index}: ${canonical(actual).slice(0, 300)}`,
      ).toBe(expected)
    },
  }
}
