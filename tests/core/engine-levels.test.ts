import { afterAll, describe, expect, it, vi } from 'vitest'
import { testPlatform } from '../fixtures/corePlatform'
import { createKChessCore } from '@kchess/native/service'
import {
  DEFAULT_ENGINE_LEVELS,
  ENGINE_LADDER,
  engineLevelLabel,
  nearestEngineLevel,
  normalizeEngineLevels,
} from '@kchess/rules/engineLevels'
import { ENGINE_LEVELS } from '@kchess/contracts/types'
import { assertSettings } from '@kchess/rules/validate'
import { DEFAULT_SETTINGS } from '@kchess/contracts/defaultSettings'

// The bundled Stockfish is found relative to the app; here that is the repository.
const core = createKChessCore(testPlatform())
// Assistance waits until the core has confirmed there is no live game (as the desktop starts it).
await core.resumeOnline()

afterAll(() => core.close())

describe('engine level ladder', () => {
  it('covers every level, weakest to strongest', () => {
    expect(ENGINE_LADDER.map((entry) => entry.id)).toEqual([...ENGINE_LEVELS])
    const elos = ENGINE_LADDER.flatMap((entry) => (entry.elo ? [entry.elo] : []))
    expect(elos).toEqual([...elos].sort((a, b) => a - b))
    for (const entry of ENGINE_LADDER)
      if (entry.uciElo) expect(entry.uciElo).toBeGreaterThanOrEqual(1320)
  })

  it('labels levels with their approximate rating', () => {
    expect(engineLevelLabel('im')).toBe('International Master · ~2400')
    expect(engineLevelLabel('max')).toBe('Stockfish Max · Full strength')
  })

  it('keeps known levels in ladder order and never returns none', () => {
    expect(normalizeEngineLevels(['gm', 'bogus', 'beginner'])).toEqual(['beginner', 'gm'])
    expect(normalizeEngineLevels([])).toEqual([...DEFAULT_ENGINE_LEVELS])
  })

  it('falls back to the closest enabled level', () => {
    expect(nearestEngineLevel('fm', ['fm', 'gm'])).toBe('fm')
    expect(nearestEngineLevel('cm', ['beginner', 'im', 'max'])).toBe('im')
    expect(nearestEngineLevel('casual', ['beginner', 'gm'])).toBe('beginner')
  })

  it('validates saved levels', () => {
    expect(
      assertSettings({ ...DEFAULT_SETTINGS, engineLevels: ['max', 'beginner'] }).engineLevels,
    ).toEqual(['beginner', 'max'])
    expect(() => assertSettings({ ...DEFAULT_SETTINGS, engineLevels: [] })).toThrow()
    expect(() => assertSettings({ ...DEFAULT_SETTINGS, engineLevels: ['low'] })).toThrow()
  })
})

describe('engine levels (bundled Stockfish)', () => {
  it.each(['beginner', 'casual', 'max'] as const)(
    '%s plays a legal move',
    async (level) => {
      const move = await core.bestMove(['e2e4'], level, { movetime: 100 })
      expect(move).toMatch(/^[a-h][1-8][a-h][1-8][nbrq]?$/)
      expect(move[1]).toMatch(/[78]/) // Black's piece or pawn
    },
    20_000,
  )

  it('the random move path returns a legal move', async () => {
    const random = vi.spyOn(Math, 'random').mockReturnValue(0)
    try {
      // The rook on a1 checks the king on h1; its only escape is h2.
      const move = await core.bestMove([], 'beginner', { fen: '7k/8/8/8/8/8/6P1/r6K w - - 0 1' })
      expect(move).toBe('h1h2')
    } finally {
      random.mockRestore()
    }
  }, 20_000)
})
