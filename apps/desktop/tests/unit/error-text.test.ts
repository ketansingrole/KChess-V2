import { describe, expect, it } from 'vitest'
import { errorText } from '../../app/utils/format'

describe('errorText', () => {
  it('drops the wrapper Electron adds to invoke errors', () => {
    const wrapped = new Error(
      "Error invoking remote method 'analysis:position-lookup': Error: Connect a Lichess account in Settings to use the opening explorer.",
    )
    expect(errorText(wrapped)).toBe(
      'Connect a Lichess account in Settings to use the opening explorer.',
    )
    expect(errorText(new Error('Plain failure.'))).toBe('Plain failure.')
    expect(errorText('text')).toBe('text')
  })
})
