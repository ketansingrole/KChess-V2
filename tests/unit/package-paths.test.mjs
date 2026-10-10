import { describe, expect, it } from 'vitest'
import { isPackageExternal, isPackageSource } from '../../tooling/package-paths.mjs'

describe('workspace package paths', () => {
  it('keeps Windows and POSIX entry modules in the build', () => {
    for (const id of [
      'D:\\a\\KChess-V2\\crates\\kchess-contracts\\ts\\apiContracts.ts',
      'D:/a/KChess-V2/crates/kchess-contracts/ts/apiContracts.ts',
      '\\\\server\\share\\apiContracts.ts',
      '/workspace/crates/kchess-contracts/ts/apiContracts.ts',
      './apiContracts.ts',
    ]) {
      expect(isPackageExternal(id)).toBe(false)
    }
    for (const id of ['@kchess/contracts', 'vue', 'node:fs']) {
      expect(isPackageExternal(id)).toBe(true)
    }
  })

  it('excludes generated output and dependencies with either path separator', () => {
    for (const separator of ['/', '\\']) {
      expect(isPackageSource(['generated', 'api.ts'].join(separator))).toBe(true)
      for (const directory of ['dist', 'node_modules']) {
        expect(isPackageSource([directory, 'api.ts'].join(separator))).toBe(false)
        expect(isPackageSource(['nested', directory, 'api.ts'].join(separator))).toBe(false)
      }
    }
    expect(isPackageSource('api.d.ts')).toBe(false)
    expect(isPackageSource('api.js')).toBe(false)
  })
})
