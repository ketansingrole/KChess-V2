import {
  closeSync,
  existsSync,
  mkdtempSync,
  openSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, it } from 'vitest'
import { replaceNative } from '../../tooling/replace-native.mjs'

it('publishes a new native file without changing the file an existing process has open', () => {
  const dir = mkdtempSync(join(tmpdir(), 'kchess-native-replacement-'))
  const target = join(dir, 'native.node')
  const source = join(dir, 'built.node')
  writeFileSync(target, 'old native code')
  writeFileSync(source, 'new native code')
  const loaded = openSync(target, 'r')
  try {
    replaceNative(source, target)
    expect(readFileSync(loaded, 'utf8')).toBe('old native code')
    expect(readFileSync(target, 'utf8')).toBe('new native code')
    expect(existsSync(`${target}.${process.pid}.tmp`)).toBe(false)
    expect(() => replaceNative(join(dir, 'missing'), target)).toThrow()
    expect(readFileSync(target, 'utf8')).toBe('new native code')
    expect(existsSync(`${target}.${process.pid}.tmp`)).toBe(false)
  } finally {
    closeSync(loaded)
    rmSync(dir, { recursive: true, force: true })
  }
})
