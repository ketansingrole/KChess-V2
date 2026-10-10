import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll } from 'vitest'
import { setPlatform, type CorePlatform, type SecretStore } from '@kchess/native/platform'

/** Tokens round-trip as base64 of the plain text; `available` can be switched per test. */
export function fakeSecrets(available = false): SecretStore & { enabled: boolean } {
  return {
    enabled: available,
    available() {
      return this.enabled
    },
    encrypt: (plain) => Buffer.from(plain).toString('base64'),
    decrypt: (base64) => Buffer.from(base64, 'base64').toString(),
  }
}

/** A headless platform rooted in a fresh temporary directory, running the bundled engine. */
export function testPlatform(overrides: Partial<CorePlatform> = {}): CorePlatform {
  let dataDir = overrides.dataDir
  if (!dataDir) {
    const created = mkdtempSync(join(tmpdir(), 'kchess-core-'))
    afterAll(() => rmSync(created, { recursive: true, force: true }))
    dataDir = created
  }
  return {
    bundledEnginePath: join(process.cwd(), 'node_modules/stockfish/bin/stockfish-19-lite.js'),
    secrets: fakeSecrets(),
    openExternal: async () => {},
    onBattery: () => false,
    ...overrides,
    dataDir,
  }
}

/** Configure the core modules for a test without creating the service. */
export function useTestPlatform(overrides: Partial<CorePlatform> = {}): CorePlatform {
  const platform = testPlatform(overrides)
  setPlatform(platform)
  return platform
}
