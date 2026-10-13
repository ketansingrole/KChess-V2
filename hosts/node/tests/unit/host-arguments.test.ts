import { expect, it } from 'vitest'
import { HOST_METHODS, validateHostArguments } from '../../src/hostArguments'

it('pins the worker-isolated host methods', () => {
  expect([...HOST_METHODS].sort()).toEqual(
    ['close', 'settings', 'suspend', 'trustEnginePath', 'voiceHistoryDocument'].sort(),
  )
})

it('validates non-core host arguments', () => {
  expect(() => validateHostArguments('settings', [])).not.toThrow()
  expect(() => validateHostArguments('close', [])).not.toThrow()
  expect(() => validateHostArguments('suspend', [])).not.toThrow()
  expect(() => validateHostArguments('voiceHistoryDocument', [])).not.toThrow()
  expect(() => validateHostArguments('settings', ['extra'])).toThrow('Invalid host arguments')
  expect(() => validateHostArguments('trustEnginePath', ['/engines/stockfish'])).not.toThrow()
  expect(() => validateHostArguments('trustEnginePath', [])).toThrow('Invalid host arguments')
  expect(() => validateHostArguments('trustEnginePath', [''])).toThrow('Invalid host arguments')
  expect(() => validateHostArguments('trustEnginePath', [123])).toThrow('Invalid host arguments')
  expect(() => validateHostArguments('trustEnginePath', ['x'.repeat(4097)])).toThrow(
    'Invalid host arguments',
  )
  expect(() => validateHostArguments('trustEnginePath', ['/engines/a', '/engines/b'])).toThrow(
    'Invalid host arguments',
  )
})
