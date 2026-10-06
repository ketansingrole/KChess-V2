import { it, expect } from 'vitest'
import { mkdtempSync, readdirSync, readFileSync, statSync, rmSync } from 'node:fs'
import { tmpdir, homedir } from 'node:os'
import { join } from 'node:path'
import {
  DiagnosticLog,
  redactDiagnostics,
  registerDiagnosticSecret,
} from '../../src/main/diagnosticLog'
import { diagnosticSessionId } from '../../src/main/diagnostics'

it('redacts registered secrets, Lichess tokens, headers, query parameters and home paths', () => {
  registerDiagnosticSecret('opaque-oauth-secret')
  const text = redactDiagnostics(
    `Bearer abc.def lip_abcdefgh lio_abcdefgh token="custom-secret" access_token=secret&code=oauth-code&state=oauth-state ${homedir()}/file opaque-oauth-secret`,
  )
  for (const secret of [
    'abc.def',
    'lip_abcdefgh',
    'lio_abcdefgh',
    'custom-secret',
    'oauth-code',
    'oauth-state',
    'opaque-oauth-secret',
    homedir(),
  ])
    expect(text).not.toContain(secret)
  expect(text).toContain('[redacted]')
})

it('redacts PKCE secrets and email addresses while keeping usernames', () => {
  const text = redactDiagnostics(
    'code_verifier=abc123 challenge=xyz code_challenge=qrs client_secret=topsecret user@example.com FEN rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR',
  )
  for (const secret of ['abc123', 'qrs', 'topsecret', 'user@example.com']) {
    expect(text).not.toContain(secret)
  }
  expect(text).toContain('[redacted]')
  // Usernames stay so logs remain correlatable per account.
  expect(redactDiagnostics('Game sync completed: Magnus')).toContain('Magnus')
})

it('issues one stable session id per launch', () => {
  expect(diagnosticSessionId()).toBe(diagnosticSessionId())
  expect(diagnosticSessionId().length).toBeGreaterThan(8)
})

it('rotates bounded logs and exports only redacted records, including large messages', () => {
  const directory = mkdtempSync(join(tmpdir(), 'kchess-log-'))
  try {
    const log = new DiagnosticLog(directory, 120)
    for (let i = 0; i < 10; i++)
      log.write('error', `record ${i} Bearer hidden-token ${'x'.repeat(40)}`)
    log.write('warn', 'x'.repeat(1000))
    const files = readdirSync(directory)
    expect(files).toHaveLength(3)
    for (const file of files) {
      expect(statSync(join(directory, file)).size).toBeLessThanOrEqual(120)
      expect(readFileSync(join(directory, file), 'utf8')).not.toContain('hidden-token')
    }
    expect(log.read().join('')).not.toContain('hidden-token')
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})
