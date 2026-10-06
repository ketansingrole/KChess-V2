import { afterAll, describe, expect, it, vi } from 'vitest'
import type { AnalysisUpdate } from '../../src/shared/types'
import { replay } from '../../src/shared/review'

// The bundled Stockfish is found relative to the app; here that is the repository.
vi.mock('electron', () => ({ app: { getAppPath: () => process.cwd() } }))
const { startAnalysis, stopAnalysis } = await import('../../src/main/analysis')

const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const ITALIAN = 'r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3'
/** White mates in one with Ra8#. */
const MATE_IN_ONE = '6k1/5ppp/8/8/8/8/8/R5K1 w - - 0 1'

const updates: AnalysisUpdate[] = []
const send = (update: AnalysisUpdate): void => void updates.push(update)
function until(test: (update: AnalysisUpdate) => boolean, ms = 20_000): Promise<AnalysisUpdate> {
  return vi.waitFor(
    () => {
      const found = updates.find(test)
      if (!found) throw new Error('not yet')
      return found
    },
    { timeout: ms, interval: 25 },
  )
}

afterAll(() => stopAnalysis(true))

describe('analysis engine (bundled Stockfish)', () => {
  it('replays castling in game history and produces legal lines for the selected position', async () => {
    const rootFen = 'r3k2r/ppp2ppp/8/8/8/8/PPP2PPP/R3K2R w KQkq - 0 1'
    const moves = ['e1h1']
    const fen = replay(rootFen, moves).at(-1)!.fen
    const id = await startAnalysis({ fen, rootFen, moves, lines: 3, infinite: true }, '', send)
    const update = await until((u) => u.id === id && u.lines.length === 3 && u.depth >= 8)
    for (const line of update.lines) {
      expect(replay(fen, line.pv)).toHaveLength(line.pv.length + 1)
    }
    stopAnalysis()
  }, 30_000)
  it('streams several scored lines for a position', async () => {
    const id = await startAnalysis({ fen: START, lines: 2, infinite: true }, '', send)
    const update = await until((u) => u.id === id && u.lines.length === 2 && u.depth >= 8)
    expect(update.fen).toBe(START)
    expect(update.done).toBe(false)
    expect(update.lines.map((line) => line.rank)).toEqual([1, 2])
    for (const line of update.lines) {
      expect(line.pv[0]).toMatch(/^[a-h][1-8][a-h][1-8]$/)
      expect(Math.abs(line.cp ?? 999)).toBeLessThan(150)
    }
  }, 30_000)

  it('drops output of a superseded search', async () => {
    updates.length = 0
    await startAnalysis({ fen: ITALIAN, lines: 1, infinite: true }, '', send)
    const id = await startAnalysis({ fen: MATE_IN_ONE, lines: 1, infinite: true }, '', send)
    const mate = await until((u) => u.id === id && u.lines[0]?.mate === 1)
    expect(mate.lines[0]!.pv[0]).toBe('a1a8')
    const afterSwitch = updates.slice(updates.findIndex((u) => u.id === id))
    expect(afterSwitch.every((u) => u.fen === MATE_IN_ONE)).toBe(true)
  }, 30_000)

  it('ends with a final update when stopped', async () => {
    updates.length = 0
    const id = await startAnalysis({ fen: ITALIAN, lines: 3, infinite: true }, '', send)
    await until((u) => u.id === id && u.lines.length === 3)
    stopAnalysis()
    const last = await until((u) => u.id === id && u.done)
    // Black to move: the scores are still from White's side, so a sound line is near equal.
    expect(last.lines).toHaveLength(3)
    expect(Math.abs(last.lines[0]!.cp ?? 999)).toBeLessThan(150)
  }, 30_000)

  it('rejects anything but a position across IPC', async () => {
    await expect(startAnalysis({ fen: 'startpos', lines: 1 }, '', send)).rejects.toThrow()
  })
})
