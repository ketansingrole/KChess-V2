import { createCipheriv, createDecipheriv, createHash, randomBytes } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import type { SecretStore } from '@kchess/native'
import { logDebug } from '@kchess/native/logger'

/** A separate Node profile uses its OS credential store; there is no plaintext fallback. */
export function systemSecrets(dataDir: string, disabled = false): SecretStore {
  const service = `KChess.Node.${createHash('sha256').update(dataDir).digest('hex')}`
  let key: Buffer | undefined
  const run = (file: string, args: string[], input?: string): string => {
    try {
      return execFileSync(file, args, {
        input,
        encoding: 'utf8',
        timeout: 15000,
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true,
      }).trim()
    } catch (cause) {
      // Child-process errors contain command arguments; never expose credential material.
      logDebug(
        'credentials',
        'OS credential operation failed:',
        cause instanceof Error ? cause.name : 'error',
      )
      // eslint-disable-next-line preserve-caught-error -- subprocess causes include credential-bearing command arguments.
      throw new Error('OS credential storage is unavailable. Unlock the system keychain and retry.')
    }
  }
  const supported =
    !disabled &&
    (process.platform === 'win32' ||
      (process.platform === 'darwin' && existsSync('/usr/bin/security')) ||
      (process.platform === 'linux' &&
        ['/usr/bin/secret-tool', '/bin/secret-tool'].some(existsSync)))
  function aesKey(create = true): Buffer {
    if (key) return key
    let stored = ''
    try {
      stored =
        process.platform === 'darwin'
          ? run('/usr/bin/security', ['find-generic-password', '-s', service, '-a', 'key', '-w'])
          : run('secret-tool', ['lookup', 'service', service, 'account', 'key'])
    } catch (cause) {
      logDebug(
        'credentials',
        'Node profile key is not available:',
        cause instanceof Error ? cause.name : 'error',
      )
    }
    if (!stored) {
      if (!create)
        throw new Error('The KChess credential key is unavailable. Reconnect your account.')
      stored = randomBytes(32).toString('hex')
      if (process.platform === 'darwin')
        run('/usr/bin/security', ['-i'], `add-generic-password -s ${service} -a key -w ${stored}\n`)
      else
        run(
          'secret-tool',
          ['store', '--label=KChess Node credentials', 'service', service, 'account', 'key'],
          stored,
        )
      const verified =
        process.platform === 'darwin'
          ? run('/usr/bin/security', ['find-generic-password', '-s', service, '-a', 'key', '-w'])
          : run('secret-tool', ['lookup', 'service', service, 'account', 'key'])
      if (verified !== stored)
        throw new Error('The OS credential key could not be saved. Retry login.')
    }
    if (!/^[0-9a-f]{64}$/.test(stored)) throw new Error('Invalid KChess credential key.')
    key = Buffer.from(stored, 'hex')
    return key
  }
  function dpapi(value: string, decrypt: boolean): string {
    const expression = decrypt
      ? '[Text.Encoding]::UTF8.GetString([Security.Cryptography.ProtectedData]::Unprotect([Convert]::FromBase64String($value), $null, [Security.Cryptography.DataProtectionScope]::CurrentUser))'
      : '[Convert]::ToBase64String([Security.Cryptography.ProtectedData]::Protect([Text.Encoding]::UTF8.GetBytes($value), $null, [Security.Cryptography.DataProtectionScope]::CurrentUser))'
    return run(
      'powershell.exe',
      [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        `Add-Type -AssemblyName System.Security; $value = [Console]::In.ReadToEnd(); ${expression}`,
      ],
      value,
    )
  }
  return {
    available: () => supported,
    encrypt(plain) {
      if (!supported) throw new Error('OS credential encryption is unavailable.')
      if (process.platform === 'win32') return dpapi(plain, false)
      const iv = randomBytes(12)
      const cipher = createCipheriv('aes-256-gcm', aesKey(), iv)
      const data = Buffer.concat([cipher.update(plain, 'utf8'), cipher.final()])
      return Buffer.concat([iv, cipher.getAuthTag(), data]).toString('base64')
    },
    decrypt(base64) {
      if (!supported) throw new Error('OS credential encryption is unavailable.')
      if (process.platform === 'win32') return dpapi(base64, true)
      const data = Buffer.from(base64, 'base64')
      if (data.length < 28) throw new Error('Invalid encrypted credential.')
      const cipher = createDecipheriv('aes-256-gcm', aesKey(false), data.subarray(0, 12))
      cipher.setAuthTag(data.subarray(12, 28))
      return Buffer.concat([cipher.update(data.subarray(28)), cipher.final()]).toString('utf8')
    },
  }
}
