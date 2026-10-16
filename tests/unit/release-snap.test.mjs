import { createHash } from 'node:crypto'
import { describe, expect, it } from 'vitest'
import { verifyReleaseSnap } from '../../tooling/download-release-snap.mjs'

const bytes = Buffer.from('release snap fixture')
const line = `${createHash('sha256').update(bytes).digest('hex')}  KChess-2026.10.9-linux-amd64.snap`

describe('published Snap verification', () => {
  it('accepts the exact versioned artifact covered by the release checksum', () => {
    expect(verifyReleaseSnap('v2026.10.9', line + '\n', bytes)).toBe(
      'KChess-2026.10.9-linux-amd64.snap',
    )
  })

  it('rejects modified bytes, a different version, missing and duplicate checksums', () => {
    expect(() => verifyReleaseSnap('v2026.10.9', line, Buffer.from('tampered'))).toThrow()
    expect(() => verifyReleaseSnap('v2026.10.10', line, bytes)).toThrow()
    expect(() => verifyReleaseSnap('v2026.10.9', '', bytes)).toThrow()
    expect(() => verifyReleaseSnap('v2026.10.9', `${line}\n${line}`, bytes)).toThrow()
    expect(() => verifyReleaseSnap('../main', line, bytes)).toThrow()
  })
})
