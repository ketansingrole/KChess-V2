import { describe, expect, it } from 'vitest'
import { createPuzzleSampler } from '../../src/services/rules'

const FEN = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const MOVES = 'e2e4 e7e5'

function line(
  id: string,
  rating: string,
  deviation: string,
  popularity: string,
  plays: string,
  themes: string,
): string {
  return `${id},${FEN},${MOVES},${rating},${deviation},${popularity},${plays},${themes},`
}

function solid(id: string, rating = 1500, themes = 'mate'): string {
  return line(id, String(rating), '50', '90', '500', themes)
}

function themeOnly(id: string, theme: string): string {
  return line(id, '1500', '100', '80', '150', theme)
}

/** The core's sampler, fed whole lines as a download delivers them. */
function sampler() {
  const inner = createPuzzleSampler()
  return {
    add: (line: string) => inner.push(Buffer.from(line + '\n')),
    get lines() {
      return inner.lines
    },
    get count() {
      return inner.count
    },
    kept: () => inner.kept(),
  }
}

describe('PuzzleSampler', () => {
  it('ignores the header row and short rows', () => {
    const sample = sampler()
    sample.add('PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes')
    sample.add('a,b,c')
    sample.add('')
    expect(sample.lines).toBe(0)
    expect(sample.kept()).toHaveLength(0)
  })

  it('throws on oversized lines', () => {
    const sample = sampler()
    expect(() => sample.add('x'.repeat(16_001))).toThrow('oversized CSV line')
  })

  it('ignores invalid ratings', () => {
    const sample = sampler()
    sample.add(line('p1', 'oops', '50', '90', '500', 'mate'))
    sample.add(line('p2', '-5', '50', '90', '500', 'mate'))
    sample.add(line('p3', '9999', '50', '90', '500', 'mate'))
    expect(sample.lines).toBe(0)
    expect(sample.kept()).toHaveLength(0)
  })

  it('keeps solid puzzles bucketed', () => {
    const sample = sampler()
    sample.add(solid('abc123'))
    expect(sample.lines).toBe(1)
    expect(sample.count).toBe(1)
    expect(sample.kept().map((row) => row.id)).toContain('abc123')
  })

  it('keeps theme puzzles even when not solid', () => {
    const sample = sampler()
    sample.add(themeOnly('theme1', 'fork'))
    expect(sample.lines).toBe(1)
    expect(sample.count).toBe(0)
    expect(sample.kept().map((row) => row.id)).toContain('theme1')
  })

  it('deduplicates puzzles kept in both buckets and themes', () => {
    const sample = sampler()
    sample.add(solid('dup1', 1500, 'fork'))
    sample.add(solid('dup1', 1500, 'fork'))
    const kept = sample.kept().filter((row) => row.id === 'dup1')
    expect(kept).toHaveLength(1)
  })

  it('bounds count by reservoir size', () => {
    const sample = sampler()
    for (let i = 0; i < 2000; i++) sample.add(solid(`p${i}`))
    expect(sample.count).toBeLessThanOrEqual(1800)
  })

  it('caps themes at 256', () => {
    const sample = sampler()
    for (let i = 0; i < 300; i++) sample.add(themeOnly(`t${i}`, `theme${i}`))
    const ids = new Set(sample.kept().map((row) => row.id))
    expect(ids.size).toBe(256)
    expect(ids.has('t0')).toBe(true)
    expect(ids.has('t299')).toBe(false)
  })

  it('rejects invalid id and theme characters', () => {
    const sample = sampler()
    sample.add(line('bad-id', '1500', '50', '90', '500', 'mate'))
    sample.add(line('x'.repeat(65), '1500', '50', '90', '500', 'mate'))
    sample.add(line('good1', '1500', '50', '90', '500', 'mate;fork'))
    sample.add(line('good2', '1500', '50', '90', '500', 'x'.repeat(257)))
    expect(sample.lines).toBe(0)
    expect(sample.kept()).toHaveLength(0)
  })
})
