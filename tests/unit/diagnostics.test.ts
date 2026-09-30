import { it, expect } from 'vitest'
import { mkdtempSync, readdirSync, readFileSync, statSync, rmSync } from 'node:fs'
import { tmpdir, homedir } from 'node:os'
import { join } from 'node:path'
import {
  DiagnosticLog,
  redactDiagnostics,
  registerDiagnosticSecret,
} from '../../src/main/diagnosticLog'

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
