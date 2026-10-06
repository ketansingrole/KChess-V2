import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DatabaseSync } from 'node:sqlite'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { migrate } from '../../src/main/migrations'
import type { VoiceAttemptInput } from '../../src/shared/types'
import { dialog } from 'electron'
import {
  clearVoiceHistory,
  exportVoiceHistory,
  saveVoiceAttempt,
  updateVoiceAttempt,
  voiceHistory,
} from '../../src/main/voiceLog'

const state = vi.hoisted(() => ({ db: null as DatabaseSync | null }))
vi.mock('electron', () => ({ dialog: { showSaveDialog: vi.fn() } }))
vi.mock('../../src/main/db', () => ({ getDb: () => state.db! }))

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
  vi.mocked(dialog.showSaveDialog).mockReset()
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

  it('clears the log and reports a cancelled export as false', async () => {
    saveVoiceAttempt(attempt())
    clearVoiceHistory()
    expect(voiceHistory(10)).toEqual([])
    vi.mocked(dialog.showSaveDialog).mockResolvedValueOnce({ canceled: true } as never)
    expect(await exportVoiceHistory()).toBe(false)
  })

  it('exports entries as JSON to the chosen path', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'kchess-voice-'))
    try {
      const filePath = join(directory, 'history.json')
      saveVoiceAttempt(attempt({ heard: 'castle kingside' }))
      vi.mocked(dialog.showSaveDialog).mockResolvedValueOnce({ canceled: false, filePath } as never)
      expect(await exportVoiceHistory()).toBe(true)
      const exported = JSON.parse(await readFile(filePath, 'utf8')) as {
        entries: { heard: string }[]
      }
      expect(exported.entries).toHaveLength(1)
      expect(exported.entries[0]!.heard).toBe('castle kingside')
    } finally {
      await rm(directory, { recursive: true, force: true })
    }
  })
})
