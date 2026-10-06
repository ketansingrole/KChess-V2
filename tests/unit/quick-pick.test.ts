import { describe, expect, it } from 'vitest'
import {
  highlightRanges,
  matchQuickPick,
  parseQuickPick,
  pushRecent,
  rankQuickPick,
} from '../../app/utils/quickPick'

describe('quick pick', () => {
  it('reads the mode from a leading prefix', () => {
    expect(parseQuickPick('> resign')).toEqual({ mode: 'commands', prefix: '>', text: 'resign' })
    expect(parseQuickPick('@magnus')).toEqual({ mode: 'players', prefix: '@', text: 'magnus' })
    expect(parseQuickPick('#sound')).toMatchObject({ mode: 'settings', text: 'sound' })
    expect(parseQuickPick('?')).toMatchObject({ mode: 'help', text: '' })
    expect(parseQuickPick(' puzzles ')).toEqual({ mode: 'all', prefix: '', text: 'puzzles' })
  })

  it('matches word starts, acronyms and every token, highlighting the label only', () => {
    expect(matchQuickPick('otb', 'Over the board')?.ranges).toEqual([
      [0, 0],
      [5, 5],
      [9, 9],
    ])
    expect(matchQuickPick('stock gm', 'Play Stockfish: Grandmaster')?.ranges).toEqual([
      [5, 9],
      [16, 16],
      [21, 21],
    ])
    expect(
      matchQuickPick('stockfish black', 'Play Stockfish: Club', '~1600 · Black')?.ranges,
    ).toEqual([[5, 13]])
    expect(matchQuickPick('stockfish rated', 'Play Stockfish: Club', '~1600 · Black')).toBeNull()
    // A subsequence must begin at a word, so scattered letters do not match.
    expect(matchQuickPick('lay', 'Puzzle history')).toBeNull()
  })

  it('ranks whole-word and label matches above partial and keyword ones', () => {
    const items = [
      { label: 'Settings', keywords: 'preferences' },
      { label: 'Play with Computer', keywords: 'stockfish bot' },
      { label: 'Play Stockfish: Beginner' },
    ]
    expect(rankQuickPick('stockfish', items).map(({ item }) => item.label)).toEqual([
      'Play Stockfish: Beginner',
      'Play with Computer',
    ])
    expect(rankQuickPick('play', items).map(({ item }) => item.label)).toEqual([
      'Play with Computer',
      'Play Stockfish: Beginner',
    ])
  })

  it('escapes labels while marking matches', () => {
    expect(highlightRanges('<b> & Bishop', [[6, 8]])).toBe('&lt;b&gt; &amp; <mark>Bis</mark>hop')
  })

  it('keeps recent entries unique and most recent first', () => {
    expect(pushRecent(['a', 'b', 'c'], 'b', 3)).toEqual(['b', 'a', 'c'])
    expect(pushRecent(['a', 'b', 'c'], 'd', 3)).toEqual(['d', 'a', 'b'])
  })
})
