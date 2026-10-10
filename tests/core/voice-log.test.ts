import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { testPlatform } from '../fixtures/corePlatform'
import { createKChessCore, type KChessCore } from '@kchess/native/service'
import type { VoiceAttemptInput } from '@kchess/contracts/types'

let core: KChessCore
let dataDir: string
beforeEach(() => {
  dataDir = mkdtempSync(join(tmpdir(), 'kchess-voice-'))
  core = createKChessCore(testPlatform({ dataDir }))
})
afterEach(async () => {
  await core.close()
  rmSync(dataDir, { recursive: true, force: true })
})

const attempt = (overrides: Partial<VoiceAttemptInput> = {}): VoiceAttemptInput => ({
  source: 'computer',
  heard: 'knight f three',
  confidence: 0.9,
  words: [{ word: 'knight', conf: 0.95 }],
  outcome: 'played',
  ...overrides,
})

describe('voice log', () => {
  it('saves, updates and lists attempts newest first', async () => {
    const first = await core.saveVoiceAttempt(attempt({ heard: 'e two to e four' }))
    const second = await core.saveVoiceAttempt(attempt({ heard: 'knight f three' }))
    expect(second).toBeGreaterThan(first)
    await core.updateVoiceAttempt(first, { outcome: 'confirmed', expected: 'Nf3' })
    const history = await core.voiceHistory(10)
    expect(history).toHaveLength(2)
    expect(history[0]!.id).toBe(second)
    expect(history[1]).toMatchObject({ id: first, outcome: 'confirmed', expected: 'Nf3' })
  })

  it('clears the log', async () => {
    await core.saveVoiceAttempt(attempt())
    await core.clearVoiceHistory()
    expect(await core.voiceHistory(10)).toEqual([])
  })

  it('exports entries as a JSON document', async () => {
    await core.saveVoiceAttempt(attempt({ heard: 'castle kingside' }))
    const exported = JSON.parse(core.voiceHistoryDocument()) as {
      entries: { heard: string; time: string }[]
    }
    expect(exported.entries).toHaveLength(1)
    expect(exported.entries[0]!.heard).toBe('castle kingside')
    expect(Date.parse(exported.entries[0]!.time)).not.toBeNaN()
  })
})
