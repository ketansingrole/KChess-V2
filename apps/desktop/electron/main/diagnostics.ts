import { app, dialog } from 'electron'
import { randomUUID } from 'node:crypto'
import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { DiagnosticLog, performanceSnapshot, startPerformanceMonitoring } from '@kchess/native'

let log: DiagnosticLog | undefined

/** Per-launch id so a shared log file can be split by session when handed over. */
let sessionId = ''
export function diagnosticSessionId(): string {
  if (!sessionId) {
    try {
      sessionId = randomUUID()
    } catch (cause) {
      console.warn('[diagnostics] Could not create session id, using fallback:', cause)
      sessionId = `${Date.now().toString(36)}-${Math.floor(Math.random() * 1e9).toString(36)}`
    }
  }
  return sessionId
}

export function setupDiagnostics(): void {
  startPerformanceMonitoring()
  try {
    log = new DiagnosticLog(join(app.getPath('userData'), 'logs'))
  } catch (error) {
    console.warn('[diagnostics] Diagnostics unavailable:', error)
  }
  /* eslint-disable logging/no-silent-catch -- These catches implement the log pipeline itself: disk-write failure must not recurse into the patched console, and fatal handling must preserve the original crash. */
  for (const level of ['log', 'info', 'warn', 'error'] as const) {
    const original = console[level].bind(console)
    console[level] = (...values: unknown[]) => {
      try {
        log?.write(level, ...values)
      } catch {
        // Disk trouble must not recursively log or prevent the original console output.
      }
      original(...values)
    }
  }
  process.on('uncaughtExceptionMonitor', (error) => {
    try {
      log?.write('fatal', error)
    } catch {
      /* Preserve the original crash. */
    }
  })
  /* eslint-enable logging/no-silent-catch */
  process.on('unhandledRejection', (reason) => {
    console.error('[diagnostics] Unhandled rejection:', reason)
  })
  console.info('[diagnostics] KChess started', {
    sessionId: diagnosticSessionId(),
    version: app.getVersion(),
    platform: process.platform,
    arch: process.arch,
    versions: process.versions,
    packaged: app.isPackaged,
  })
}

export async function exportDiagnostics(): Promise<boolean> {
  const result = await dialog.showSaveDialog({
    title: 'Export KChess diagnostics',
    defaultPath: `kchess-diagnostics-${new Date().toISOString().slice(0, 10)}.json`,
    filters: [{ name: 'Diagnostic report', extensions: ['json'] }],
  })
  if (result.canceled || !result.filePath) return false
  await writeFile(
    result.filePath,
    JSON.stringify(
      {
        generatedAt: new Date().toISOString(),
        sessionId: diagnosticSessionId(),
        version: app.getVersion(),
        platform: process.platform,
        arch: process.arch,
        versions: process.versions,
        logs: log?.read() ?? [],
        performance: performanceSnapshot(),
      },
      null,
      2,
    ),
    { mode: 0o600 },
  )
  return true
}
