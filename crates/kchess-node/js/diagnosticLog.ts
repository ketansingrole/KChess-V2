import {
  appendFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  statSync,
  unlinkSync,
} from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { format } from 'node:util'

const secrets = new Set<string>()
/** Register tokens when they enter the main process, before any error can print them. */
export function registerDiagnosticSecret(secret: string): void {
  if (secret) secrets.add(secret)
}

export function redactDiagnostics(value: string): string {
  let text = value.replaceAll(homedir(), '~')
  for (const secret of secrets) text = text.replaceAll(secret, '[redacted]')
  return (
    text
      .replace(/\bBearer\s+[A-Za-z0-9._~+/-]+=*/gi, 'Bearer [redacted]')
      .replace(/\b(?:lip|lio)_[A-Za-z0-9_-]+\b/g, '[redacted]')
      // OAuth/PKCE and credential keys: `code` alone must not match `code_verifier`
      // as a prefix without its suffix, so list the longer keys explicitly.
      .replace(
        /(["']?(?:access_token|refresh_token|id_token|token|password|authorization|cookie|code_verifier|code_challenge|client_secret|verifier|challenge|code|state)["']?\s*[:=]\s*)(["']?)[^\s,"'&}]+\2/gi,
        '$1[redacted]',
      )
      // Email addresses are PII; usernames stay so logs remain correlatable.
      .replace(/[\w.+-]+@[\w-]+\.[\w.]+/g, '[redacted]')
  )
}

/** Bounded local logs; never include database contents or request bodies. */
export class DiagnosticLog {
  constructor(
    private readonly directory: string,
    private readonly maxBytes = 512 * 1024,
  ) {
    mkdirSync(directory, { recursive: true, mode: 0o700 })
  }

  private path(index = 0): string {
    return join(this.directory, index ? `kchess.${index}.log` : 'kchess.log')
  }

  write(level: string, ...values: unknown[]): void {
    const message = redactDiagnostics(format(...values))
    // A single unexpected object must not defeat the file-size bound.
    const line = Buffer.from(`${new Date().toISOString()} ${level.toUpperCase()} ${message}\n`)
    const entry = line.subarray(0, this.maxBytes)
    if (existsSync(this.path()) && statSync(this.path()).size + entry.length > this.maxBytes) {
      if (existsSync(this.path(2))) unlinkSync(this.path(2))
      if (existsSync(this.path(1))) renameSync(this.path(1), this.path(2))
      renameSync(this.path(), this.path(1))
    }
    appendFileSync(this.path(), entry, { mode: 0o600 })
  }

  read(): string[] {
    return [2, 1, 0]
      .filter((index) => existsSync(this.path(index)))
      .map((index) => redactDiagnostics(readFileSync(this.path(index), 'utf8')))
  }
}
