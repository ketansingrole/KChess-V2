import { describe, expect, it } from 'vitest'
import { PuzzleSampler } from '../../src/core/puzzleSampler'

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

describe('PuzzleSampler', () => {
  it('ignores the header row and short rows', () => {
    const sampler = new PuzzleSampler()
    sampler.add('PuzzleId,FEN,Moves,Rating,RatingDeviation,Popularity,NbPlays,Themes')
    sampler.add('a,b,c')
    sampler.add('')
    expect(sampler.lines).toBe(0)
    expect(sampler.kept()).toHaveLength(0)
  })

  it('throws on oversized lines', () => {
    const sampler = new PuzzleSampler()
    expect(() => sampler.add('x'.repeat(16_001))).toThrow()
  })

  it('ignores invalid ratings', () => {
    const sampler = new PuzzleSampler()
    sampler.add(line('p1', 'oops', '50', '90', '500', 'mate'))
    sampler.add(line('p2', '-5', '50', '90', '500', 'mate'))
    sampler.add(line('p3', '9999', '50', '90', '500', 'mate'))
    expect(sampler.lines).toBe(0)
    expect(sampler.kept()).toHaveLength(0)
  })

  it('keeps solid puzzles bucketed', () => {
    const sampler = new PuzzleSampler()
    sampler.add(solid('abc123'))
    expect(sampler.lines).toBe(1)
    expect(sampler.count).toBe(1)
    expect(sampler.kept().map((row) => row.id)).toContain('abc123')
  })

  it('keeps theme puzzles even when not solid', () => {
    const sampler = new PuzzleSampler()
    sampler.add(themeOnly('theme1', 'fork'))
    expect(sampler.lines).toBe(1)
    expect(sampler.count).toBe(0)
    expect(sampler.kept().map((row) => row.id)).toContain('theme1')
  })

  it('deduplicates puzzles kept in both buckets and themes', () => {
    const sampler = new PuzzleSampler()
    sampler.add(solid('dup1', 1500, 'fork'))
    sampler.add(solid('dup1', 1500, 'fork'))
    const kept = sampler.kept().filter((row) => row.id === 'dup1')
    expect(kept).toHaveLength(1)
  })

  it('bounds count by reservoir size', () => {
    const sampler = new PuzzleSampler()
    for (let i = 0; i < 2000; i++) sampler.add(solid(`p${i}`))
    expect(sampler.count).toBeLessThanOrEqual(1800)
  })

  it('caps themes at 256', () => {
    const sampler = new PuzzleSampler()
    for (let i = 0; i < 300; i++) sampler.add(themeOnly(`t${i}`, `theme${i}`))
    const ids = new Set(sampler.kept().map((row) => row.id))
    expect(ids.size).toBe(256)
    expect(ids.has('t0')).toBe(true)
    expect(ids.has('t299')).toBe(false)
  })

  it('rejects invalid id and theme characters', () => {
    const sampler = new PuzzleSampler()
    sampler.add(line('bad-id', '1500', '50', '90', '500', 'mate'))
    sampler.add(line('x'.repeat(65), '1500', '50', '90', '500', 'mate'))
    sampler.add(line('good1', '1500', '50', '90', '500', 'mate;fork'))
    sampler.add(line('good2', '1500', '50', '90', '500', 'x'.repeat(257)))
    expect(sampler.lines).toBe(0)
    expect(sampler.kept()).toHaveLength(0)
  })
})
