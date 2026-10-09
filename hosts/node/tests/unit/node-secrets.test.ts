import { beforeEach, expect, it, vi } from 'vitest'
import { systemSecrets } from '../../src/secrets'
const exec = vi.hoisted(() => vi.fn())
vi.mock('node:child_process', () => ({ default: { execFileSync: exec }, execFileSync: exec }))
vi.mock('node:fs', () => ({ default: { existsSync: () => true }, existsSync: () => true }))

let key: string | undefined
beforeEach(() => {
  key = undefined
  vi.stubGlobal('process', { ...process, platform: 'darwin' })
  exec.mockImplementation((_file: string, args: string[], options: { input?: string }) => {
    if (args[0] === 'find-generic-password') {
      if (!key) throw new Error('No key')
      return key
    }
    if (args[0] === '-i') {
      key = /-w ([0-9a-f]{64})/.exec(options.input ?? '')?.[1]
      return ''
    }
    throw new Error('Unexpected credential command')
  })
})
it('encrypts with the per-profile OS key and decrypts in a new host', () => {
  const secrets = systemSecrets('/tmp/profile')
  const encrypted = secrets.encrypt('test-token')
  expect(Buffer.from(encrypted, 'base64').toString()).not.toContain('test-token')
  expect(systemSecrets('/tmp/profile').decrypt(encrypted)).toBe('test-token')
  expect(exec.mock.calls.flatMap((call) => call[1])).not.toContain('test-token')
})
it('does not generate a replacement key when decrypting an existing credential', () => {
  const secrets = systemSecrets('/tmp/profile')
  expect(() => secrets.decrypt(Buffer.alloc(29).toString('base64'))).toThrow('Reconnect')
  expect(exec.mock.calls.some((call) => call[1][0] === '-i')).toBe(false)
})
it('rejects modified authenticated ciphertext', () => {
  const secrets = systemSecrets('/tmp/profile')
  const data = Buffer.from(secrets.encrypt('test-token'), 'base64')
  data[data.length - 1] = data[data.length - 1]! ^ 1
  expect(() => secrets.decrypt(data.toString('base64'))).toThrow()
})
it('disables credentials without invoking the system keychain', () => {
  const secrets = systemSecrets('/tmp/profile', true)
  expect(secrets.available()).toBe(false)
  expect(() => secrets.encrypt('test-token')).toThrow('unavailable')
  expect(exec).not.toHaveBeenCalled()
})
it('does not expose command arguments or keys when the OS store fails', () => {
  exec.mockImplementation(() => {
    throw new Error('Command failed with secret-key-123')
  })
  expect(() => systemSecrets('/tmp/profile').encrypt('test-token')).toThrow(
    'OS credential storage is unavailable',
  )
  expect(() => systemSecrets('/tmp/profile').encrypt('test-token')).not.toThrow('secret-key-123')
})
