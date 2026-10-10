import { nativeCallSync } from '@kchess/native/nativeCore'
import { setPlatform, type CorePlatform } from '@kchess/native/platform'
import { testPlatform } from './corePlatform'

/**
 * Raw SQL on the native core's `kchess.db`, for tests: `exec` runs statements, `prepare` binds
 * parameters for `all`, `get` and `run`. The statements go through the core's test-only
 * `store.debug.*` methods, which the application never calls.
 */
export interface TestDatabase {
  exec(sql: string): void
  prepare(sql: string): {
    all(...params: unknown[]): Record<string, unknown>[]
    get(...params: unknown[]): Record<string, unknown> | undefined
    run(...params: unknown[]): void
  }
}

const database: TestDatabase = {
  exec(sql) {
    nativeCallSync('store.debug.exec', sql, [])
  },
  prepare(sql) {
    return {
      all: (...params) =>
        nativeCallSync<Record<string, unknown>[]>('store.debug.query', sql, params),
      get: (...params) =>
        nativeCallSync<Record<string, unknown>[]>('store.debug.query', sql, params)[0],
      run: (...params) => {
        nativeCallSync('store.debug.exec', sql, params)
      },
    }
  },
}

/**
 * Point the core at a profile (a fresh temporary one unless `dataDir` is given). The database opens,
 * and is migrated, on first use; `closeNativeCore` closes it, and a later call reopens it.
 */
export function useTestDatabase(overrides: Partial<CorePlatform> = {}): TestDatabase {
  const platform = testPlatform(overrides)
  setPlatform(platform)
  nativeCallSync('store.open')
  return database
}
