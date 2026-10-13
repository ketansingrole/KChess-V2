import { describe, expect, it } from 'vitest'
import { CORE_CONTRACTS, validateCoreArguments } from '@kchess/contracts/apiContracts'

/**
 * The headless argument contracts for the study library's incremental flag
 * (`studyCommand(command, incremental?)`, `kchess_domain::misc::contracts`):
 * the arity the new argument added and the boolean check on it. Mirrors the
 * desktop's `validateIpcArguments` pinning in
 * `apps/desktop/tests/unit/ipc-contracts.test.ts`, which has no core
 * equivalent; the service test in `core-library.test.ts` covers the flag
 * end to end but not the contract itself.
 */
describe('core argument contracts', () => {
  it('pins the studyCommand incremental flag contract', () => {
    expect(CORE_CONTRACTS.studyCommand).toEqual({ min: 1, max: 2 })
    const command = { op: 'save', name: 'Study', pgn: '1. e4 *' }
    expect(() => validateCoreArguments('studyCommand', [command])).not.toThrow()
    expect(() => validateCoreArguments('studyCommand', [command, true])).not.toThrow()
    expect(() => validateCoreArguments('studyCommand', [command, false])).not.toThrow()
    expect(() => validateCoreArguments('studyCommand', [command, 'yes' as never])).toThrow()
    expect(() => validateCoreArguments('studyCommand', [])).toThrow('argument count')
    expect(() => validateCoreArguments('studyCommand', [command, true, 'extra'])).toThrow(
      'argument count',
    )
  })

  it('pins engine teardown and empty-sync arities', () => {
    expect(CORE_CONTRACTS.stopAnalysis).toEqual({ min: 0, max: 1 })
    expect(() => validateCoreArguments('stopAnalysis', [])).not.toThrow()
    expect(() => validateCoreArguments('stopAnalysis', [true])).not.toThrow()
    expect(() => validateCoreArguments('stopAnalysis', ['yes' as never])).toThrow()
    expect(() => validateCoreArguments('stopAnalysis', [true, false])).toThrow('argument count')
    expect(() => validateCoreArguments('syncGames', [])).not.toThrow()
    expect(() => validateCoreArguments('syncGames', ['alice'])).not.toThrow()
    expect(() => validateCoreArguments('syncGames', ['alice', 'extra'])).toThrow('argument count')
  })
})
