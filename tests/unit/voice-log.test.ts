import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DatabaseSync } from 'node:sqlite'
import { migrate } from '../../src/core/migrations'
import type { VoiceAttemptInput } from '../../src/shared/types'
import {
  clearVoiceHistory,
  saveVoiceAttempt,
  updateVoiceAttempt,
  voiceHistory,
  voiceHistoryDocument,
} from '../../src/core/voiceLog'

const state = vi.hoisted(() => ({ db: null as DatabaseSync | null }))
vi.mock('../../src/core/db', () => ({ getDb: () => state.db! }))

const attempt = (overrides: Partial<VoiceAttemptInput> = {}): VoiceAttemptInput => ({
  source: 'computer',
  heard: 'knight f three',
  confidence: 0.9,
  words: [{ word: 'knight', conf: 0.95 }],
  outcome: 'played',
  ...overrides,
})

beforeEach(() => {
  state.db = new DatabaseSync(':memory:')
  migrate(state.db)
})

afterEach(() => {
  state.db?.close()
  state.db = null
})

describe('voice log', () => {
  it('saves, updates and lists attempts newest first', () => {
    const first = saveVoiceAttempt(attempt({ heard: 'e two to e four' }))
    const second = saveVoiceAttempt(attempt({ heard: 'knight f three' }))
    expect(second).toBeGreaterThan(first)
    updateVoiceAttempt(first, { outcome: 'confirmed', expected: 'Nf3' })
    const history = voiceHistory(10)
    expect(history).toHaveLength(2)
    expect(history[0]!.id).toBe(second)
    expect(history[1]).toMatchObject({ id: first, outcome: 'confirmed', expected: 'Nf3' })
  })

  it('clears the log', () => {
    saveVoiceAttempt(attempt())
    clearVoiceHistory()
    expect(voiceHistory(10)).toEqual([])
  })

  it('exports entries as a JSON document', () => {
    saveVoiceAttempt(attempt({ heard: 'castle kingside' }))
    const exported = JSON.parse(voiceHistoryDocument()) as {
      entries: { heard: string; time: string }[]
    }
    expect(exported.entries).toHaveLength(1)
    expect(exported.entries[0]!.heard).toBe('castle kingside')
    expect(Date.parse(exported.entries[0]!.time)).not.toBeNaN()
  })
})
