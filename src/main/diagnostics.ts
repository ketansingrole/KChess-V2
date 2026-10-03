import { app, dialog } from 'electron'
import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { startPerformanceMonitoring, performanceSnapshot } from './performance'
import { DiagnosticLog } from './diagnosticLog'

let log: DiagnosticLog | undefined

export function setupDiagnostics(): void {
  startPerformanceMonitoring()
  try {
    log = new DiagnosticLog(join(app.getPath('userData'), 'logs'))
  } catch (error) {
    console.warn('Diagnostics unavailable:', error)
  }
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
  process.on('unhandledRejection', (reason) => {
    console.error('Unhandled rejection:', reason)
  })
  console.info('KChess started', {
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
