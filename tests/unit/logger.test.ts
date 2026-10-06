import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  errorSummary,
  isExpectedCancellation,
  logDebug,
  logError,
  logInfo,
  logWarn,
  truncateForLog,
  uciCommandName,
} from '../../src/main/logger'
import { LichessError } from '../../src/shared/lichessError'
import { SearchCancelled } from '../../src/main/uci'

afterEach(() => {
  vi.restoreAllMocks()
})

describe('main logger', () => {
  it('prefixes entries with the scope and delegates to console', () => {
    const info = vi.spyOn(console, 'info').mockImplementation(() => {})
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const log = vi.spyOn(console, 'log').mockImplementation(() => {})

    logInfo('startup', 'ready')
    logWarn('app-updates', 'retrying:', 503)
    logError('renderer', 'crashed:', { reason: 'oom' })
    logDebug('lichess-cache', 'write skipped')

    expect(info).toHaveBeenCalledOnce()
    expect(info.mock.calls[0]![0]).toBe('[startup] ready')
    expect(warn.mock.calls[0]![0]).toContain('[app-updates] retrying: 503')
    expect(error.mock.calls[0]![0]).toContain('[renderer] crashed:')
    expect(log.mock.calls[0]![0]).toBe('[lichess-cache] write skipped')
  })

  it('truncates long payloads so one entry cannot flood the bounded files', () => {
    expect(truncateForLog('short')).toBe('short')
    const long = truncateForLog('x'.repeat(500))
    expect(long.length).toBeLessThan(500)
    expect(long).toContain('more chars')
    expect(truncateForLog('a  b\nc')).toBe('a b c')
  })

  it('summarizes errors with status, endpoint, and code', () => {
    const lichess = new LichessError(429, 'GET /api/user/x', 'slow down')
    expect(errorSummary(lichess)).toContain('status=429')
    expect(errorSummary(lichess)).toContain('endpoint=GET /api/user/x')
    expect(errorSummary('plain failure')).toBe('plain failure')
    const coded = Object.assign(new Error('timeout'), { code: 'ETIMEDOUT' })
    expect(errorSummary(coded)).toContain('code=ETIMEDOUT')
  })

  it('classifies cancellations as routine control flow', () => {
    expect(isExpectedCancellation(new SearchCancelled())).toBe(true)
    expect(isExpectedCancellation(new DOMException('aborted', 'AbortError'))).toBe(true)
    expect(isExpectedCancellation(new Error('Engine search superseded.'))).toBe(true)
    expect(isExpectedCancellation(new Error('offline'))).toBe(false)
  })

  it('names UCI commands without the position payload', () => {
    expect(uciCommandName('go movetime 500')).toBe('go')
    const position = uciCommandName(
      'position fen rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1 moves e2e4 e7e5',
    )
    expect(position).toContain('position')
    expect(position).toContain('moves(2)')
    expect(position).not.toContain('e2e4')
  })
})
