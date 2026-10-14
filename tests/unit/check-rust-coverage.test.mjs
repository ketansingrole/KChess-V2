import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, expect, it } from 'vitest'
import { belowFloors, hasDocTests } from '../../tooling/check-rust-coverage.mjs'

const roots = []
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true })
})
function crate(source) {
  const root = mkdtempSync(join(tmpdir(), 'kchess-doctests-'))
  roots.push(root)
  mkdirSync(join(root, 'demo', 'src'), { recursive: true })
  writeFileSync(join(root, 'demo', 'src', 'lib.rs'), source)
  return root
}

// `check` runs the Rust tests once, under coverage; llvm-cov skips doc-tests on stable, so the
// coverage step must notice them and run `cargo test --doc` itself.
it('finds doc-tests in Rust doc comments, and only there', async () => {
  expect(await hasDocTests(crate('/// Adds.\npub fn add() {}\n// ```not a doc comment\n'))).toBe(
    false,
  )
  expect(await hasDocTests(crate('/// Adds.\n/// ```\n/// assert_eq!(1, 1);\n/// ```\n'))).toBe(
    true,
  )
  expect(await hasDocTests(crate('//! Crate.\n//! ```rust\n//! demo::add();\n//! ```\n'))).toBe(
    true,
  )
})

it('reports every metric below its floor', () => {
  expect(
    belowFloors(
      { lines: 70, regions: 50, functions: 62 },
      { lines: 65, regions: 61, functions: 62 },
    ),
  ).toEqual(['regions coverage 50.00% is below the 61% floor'])
})
