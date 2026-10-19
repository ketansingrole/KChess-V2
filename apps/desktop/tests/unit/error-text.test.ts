import { describe, expect, it } from 'vitest'
import { errorText } from '../../app/utils/format'
import { invokeErrorText } from '../../contracts/ipc'

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

describe('invokeErrorText', () => {
  it('is what the preload rejects with, so no invoke wrapper reaches a page', () => {
    expect(
      invokeErrorText(
        new Error("Error invoking remote method 'puzzle:next': Error: Lichess is unreachable."),
      ),
    ).toBe('Lichess is unreachable.')
    expect(
      invokeErrorText(
        "Error invoking remote method 'lichess:profile': LichessError: Lichess 404 from /api/user/x",
      ),
    ).toBe('Lichess 404 from /api/user/x')
    expect(errorText).toBe(invokeErrorText)
  })
})
