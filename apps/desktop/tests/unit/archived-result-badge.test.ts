import { describe, expect, it } from 'vitest'
import { STANDARD_SETUP } from '@kchess/rules/variant'
import type { ArchivedGame } from '../../app/stores/gameArchive'
import { archivedResultBadge } from '../../app/utils/games'

const game = (overrides: Partial<ArchivedGame>): ArchivedGame => ({
  id: 'g',
  source: 'computer',
  startedAt: 0,
  updatedAt: 0,
  white: 'You',
  black: 'Stockfish (Club Player)',
  result: '*',
  reason: 'In progress',
  finished: true,
  setup: STANDARD_SETUP,
  moves: ['e2e4'],
  timeControl: '-',
  ...overrides,
})
const label = (overrides: Partial<ArchivedGame>) => archivedResultBadge(game(overrides)).label

describe('archived game result badge', () => {
  it('reads computer games from the player’s side', () => {
    expect(label({ result: '1-0' })).toBe('Win')
    expect(label({ result: '0-1', reason: 'You resigned' })).toBe('Loss')
    expect(label({ white: 'Stockfish (Club Player)', black: 'You', result: '1-0' })).toBe('Loss')
    expect(label({ white: 'Stockfish (Club Player)', black: 'You', result: '0-1' })).toBe('Win')
    expect(label({ result: '1/2-1/2' })).toBe('Draw')
    expect(archivedResultBadge(game({ result: '1-0' })).color).toBe('success')
    expect(archivedResultBadge(game({ result: '0-1' })).color).toBe('error')
  })

  it('names the winning colour of an over-the-board game', () => {
    const board = { source: 'board', white: 'White', black: 'Black' } as const
    expect(label({ ...board, result: '1-0' })).toBe('White won')
    expect(label({ ...board, result: '0-1' })).toBe('Black won')
    expect(label({ ...board, result: '1/2-1/2' })).toBe('Draw')
  })

  it('separates unfinished, running and clock-only sessions from results', () => {
    expect(label({ result: '*', finished: true })).toBe('Unfinished')
    expect(label({ result: '*', finished: false })).toBe('In progress')
    expect(label({ source: 'clock', moves: [] })).toBe('Clock')
  })
})
