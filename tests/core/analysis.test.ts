import { describe, expect, it } from 'vitest'
import { parseInfo } from '@kchess/rules/uciInfo'
import { assertAnalysisRequest } from '@kchess/rules/validate'
import {
  addMove,
  clearPvSanCache,
  deleteAt,
  formatEval,
  lineEnd,
  newTree,
  nodeAt,
  onMainline,
  promoteToMainline,
  pvSan,
  treeFromPgn,
  treeToPgn,
  winningChances,
} from '@kchess/rules/analysisTree'
import {
  castlingAvailable,
  EMPTY_BOARD,
  enPassantSquares,
  positionProblem,
  setupFen,
  setupFromFen,
  START_SETUP,
  withPiece,
} from '@kchess/rules/boardEditor'
import {
  ANALYSIS_GRAMMAR,
  EDITOR_GRAMMAR,
  spokenAnalysisCommand,
  spokenEdit,
} from '@kchess/rules/voiceCommands'

describe('UCI info lines', () => {
  it('reads a scored principal variation from White’s point of view', () => {
    const line =
      'info depth 18 seldepth 24 multipv 2 score cp 31 nodes 912 nps 450000 hashfull 12 tbhits 0 time 2 pv e2e4 e7e5 g1f3'
    expect(parseInfo(line, true)).toEqual({
      line: { rank: 2, depth: 18, cp: 31, pv: ['e2e4', 'e7e5', 'g1f3'] },
      nps: 450000,
    })
    // UCI scores are for the side to move: Black being +31 means White is −31.
    expect(parseInfo(line, false)?.line.cp).toBe(-31)
  })
  it('turns mate scores around for Black too', () => {
    expect(parseInfo('info depth 5 score mate 2 pv d8h4', false)?.line.mate).toBe(-2)
    expect(parseInfo('info depth 5 score mate -1 pv e1e2', true)?.line.mate).toBe(-1)
  })
  it('skips progress lines, bound scores and text', () => {
    expect(parseInfo('info depth 12 currmove e2e4 currmovenumber 1', true)).toBeUndefined()
    expect(parseInfo('info depth 12 score cp 40 lowerbound nodes 1 pv e2e4', true)).toBeUndefined()
    expect(parseInfo('info string NNUE evaluation using nn.nnue', true)).toBeUndefined()
    expect(parseInfo('info depth 0 score mate 0', true)).toBeUndefined()
    expect(parseInfo('bestmove e2e4', true)).toBeUndefined()
    expect(parseInfo('', true)).toBeUndefined()
    expect(parseInfo('  bestmove e2e4  ', true)).toBeUndefined()
    // Indented engine output keeps the old trimmed semantics.
    expect(parseInfo('  info depth 5 score mate 2 pv d8h4  ', false)?.line.mate).toBe(-2)
  })
  it('steps over the three values of wdl', () => {
    expect(parseInfo('info depth 9 score cp 12 wdl 300 500 200 pv d2d4', true)?.line).toEqual({
      rank: 1,
      depth: 9,
      cp: 12,
      pv: ['d2d4'],
    })
  })
})

describe('analysis requests across IPC', () => {
  it('accepts a FEN with 1–5 lines and rejects anything else', () => {
    const fen = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
    expect(assertAnalysisRequest({ fen, lines: 3 })).toEqual({ fen, lines: 3 })
    expect(() => assertAnalysisRequest({ fen, lines: 9 })).toThrow()
    expect(() => assertAnalysisRequest({ fen: `${fen}\ngo infinite`, lines: 1 })).toThrow()
    expect(() => assertAnalysisRequest({ fen: 'startpos', lines: 1 })).toThrow()
  })
})

describe('analysis move tree', () => {
  it('keeps variations and re-uses moves already tried', () => {
    const root = newTree()
    const e4 = addMove(root, '', 'e2e4')!
    addMove(root, e4, 'e7e5')
    const c5 = addMove(root, e4, 'e7c5')
    expect(c5).toBeUndefined() // illegal
    const sicilian = addMove(root, e4, 'c7c5')!
    expect(addMove(root, '', 'e2e4')).toBe('e2e4')
    expect(root.children).toHaveLength(1)
    expect(nodeAt(root, e4).children.map((n) => n.san)).toEqual(['e5', 'c5'])
    expect(onMainline(root, 'e2e4 e7e5')).toBe(true)
    expect(onMainline(root, sicilian)).toBe(false)
    expect(lineEnd(root, '')).toBe('e2e4 e7e5')
  })
  it('promotes a variation and deletes a line', () => {
    const root = newTree()
    addMove(root, '', 'e2e4')
    addMove(root, 'e2e4', 'e7e5')
    addMove(root, 'e2e4', 'c7c5')
    promoteToMainline(root, 'e2e4 c7c5')
    expect(lineEnd(root, '')).toBe('e2e4 c7c5')
    expect(deleteAt(root, 'e2e4 c7c5')).toBe('e2e4')
    expect(lineEnd(root, '')).toBe('e2e4 e7e5')
  })
  it('round-trips PGN with variations and a custom start', () => {
    const pgn = '1. e4 e5 (1... c5 2. Nf3) 2. Nf3 Nc6 *'
    const root = treeFromPgn(pgn)!
    expect(lineEnd(root, '')).toBe('e2e4 e7e5 g1f3 b8c6')
    expect(nodeAt(root, 'e2e4').children.map((n) => n.san)).toEqual(['e5', 'c5'])
    const again = treeFromPgn(treeToPgn(root))!
    expect(treeToPgn(again)).toBe(treeToPgn(root))

    const fen = '4k3/8/8/8/8/8/4P3/4K3 w - - 0 1'
    const endgame = newTree(fen)
    addMove(endgame, '', 'e2e4')
    const text = treeToPgn(endgame)
    expect(text).toContain(`[FEN "${fen}"]`)
    expect(treeFromPgn(text)!.fen).toBe(fen)
  })
  it('preserves headers, result and annotations through PGN round trips', () => {
    const pgn =
      '[Event "Study"]\n[White "Alice"]\n[Black "Bob"]\n[Result "1-0"]\n\n{Introduction} 1. e4 $1 {A useful move [%clk 0:10:00]} e5 (1... c5 {Sicilian}) 1-0'
    const root = treeFromPgn(pgn)!
    const exported = treeToPgn(root)
    expect(exported).toContain('[White "Alice"]')
    expect(exported).toContain('[Result "1-0"]')
    expect(exported).toContain('Introduction')
    expect(exported).toContain('$1')
    expect(exported).toContain('[%clk 0:10:00]')
    expect(exported).toContain('Sicilian')
    expect(treeToPgn(treeFromPgn(exported)!)).toBe(exported)
    expect(treeFromPgn('1. e4 e5 2. Ke3 *')).toBeUndefined()
    expect(treeFromPgn('[Event "One"]\n1. e4 *\n\n[Event "Two"]\n1. d4 *')).toBeUndefined()
  })
  it('numbers moves from a position with Black to move', () => {
    const fen = 'rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1'
    expect(pvSan(fen, ['e7e5', 'g1f3', 'zzzz']).map((m) => m.label)).toEqual(['1… e5', '2. Nf3'])
  })
  it('reuses the SAN of an unchanged engine line instead of replaying it', () => {
    clearPvSanCache()
    const fen = 'rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1'
    const pv = ['e7e5', 'g1f3']
    const first = pvSan(fen, pv)
    // Live engine ticks resend the same PV at deeper depths: no new replay.
    expect(pvSan(fen, [...pv])).toBe(first)
    expect(pvSan(fen, [...pv, 'b8c6'], 2)).toBe(first)
    expect(pvSan(fen, [...pv, 'b8c6'])).not.toBe(first)
    expect(pvSan(fen, [...pv]).map((m) => m.san)).toEqual(['e5', 'Nf3'])
  })
  it('formats evaluations like Lichess', () => {
    expect(formatEval({ cp: 34 })).toBe('+0.3')
    expect(formatEval({ cp: -120 })).toBe('−1.2')
    expect(formatEval({ cp: 0 })).toBe('0.0')
    expect(formatEval({ mate: 3 })).toBe('#3')
    expect(formatEval({ mate: -2 })).toBe('#−2')
    expect(winningChances({ mate: -4 })).toBe(-1)
    expect(winningChances({ cp: 0 })).toBe(0)
    expect(winningChances({ cp: 300 })).toBeGreaterThan(0.5)
  })
})

describe('board editor', () => {
  it('writes only the castling rights the pieces allow', () => {
    expect(setupFen(START_SETUP)).toBe('rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1')
    const noRook = withPiece(START_SETUP.board, 'h1', undefined)
    expect(castlingAvailable(noRook)).toEqual({ K: false, Q: true, k: true, q: true })
    expect(setupFen({ ...START_SETUP, board: noRook })).toContain(' w Qkq - ')
  })
  it('places and removes pieces in the board field', () => {
    const board = withPiece(EMPTY_BOARD, 'e4', { color: 'white', role: 'knight' })
    expect(board).toBe('8/8/8/8/4N3/8/8/8')
    expect(withPiece(board, 'a8', { color: 'black', role: 'king' })).toBe('k7/8/8/8/4N3/8/8/8')
    expect(withPiece(board, 'e4', undefined)).toBe(EMPTY_BOARD)
  })
  it('offers en passant only where a pawn just could have double-stepped', () => {
    const board = '4k3/8/8/3pP3/8/8/8/4K3'
    expect(enPassantSquares(board, 'white')).toEqual(['d6'])
    expect(enPassantSquares(board, 'black')).toEqual([])
    expect(setupFen({ ...START_SETUP, board, ep: 'd6' })).toBe('4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1')
  })
  it('explains why a position cannot be analysed', () => {
    expect(positionProblem(setupFen({ ...START_SETUP, board: EMPTY_BOARD }))).toMatch(/empty/)
    expect(positionProblem('4k3/8/8/8/8/8/8/8 w - - 0 1')).toMatch(/one king/)
    expect(positionProblem('P3k3/8/8/8/8/8/8/4K3 w - - 0 1')).toMatch(/Pawns/)
    expect(positionProblem('4k3/8/8/8/8/8/8/4KR2 w - - 0 1')).toBeUndefined()
    expect(positionProblem('4k3/4R3/8/8/8/8/8/4K3 w - - 0 1')).toMatch(/not to move/)
  })
  it('reads a pasted FEN', () => {
    expect(setupFromFen('8/8/8/8/8/8/8/K6k b - - 3 40')).toMatchObject({
      board: '8/8/8/8/8/8/8/K6k',
      turn: 'black',
    })
    expect(setupFromFen('not a fen')).toBeUndefined()
  })
})

describe('voice in the editor and on the analysis board', () => {
  it.each([
    ['white knight F three', { kind: 'place', square: 'f3', color: 'white', role: 'knight' }],
    ['black king on echo eight', { kind: 'place', square: 'e8', color: 'black', role: 'king' }],
    ['queen D one', { kind: 'place', square: 'd1', color: 'black', role: 'queen' }],
    ['white pawn eight three', { kind: 'place', square: 'a3', color: 'white', role: 'pawn' }],
    ['remove E four', { kind: 'remove', square: 'e4' }],
    ['clear board', { kind: 'clear' }],
    ['starting position', { kind: 'start' }],
    ['black to move', { kind: 'turn', color: 'black' }],
    ['analyze', { kind: 'analyze' }],
  ])('editor understands %s', (text, action) => {
    expect(spokenEdit(text, 'black')).toEqual(action)
  })
  it.each(['white knight', 'knight bishop e four', 'e four', 'hello'])(
    'editor rejects %s',
    (text) => {
      expect(spokenEdit(text)).toBeUndefined()
    },
  )
  it('steps through analysis by voice', () => {
    expect(spokenAnalysisCommand('[unk] back')).toBe('back')
    expect(spokenAnalysisCommand('next')).toBe('forward')
    expect(spokenAnalysisCommand('best move')).toBe('best')
    expect(spokenAnalysisCommand('knight f three')).toBeUndefined()
  })
  it('keeps the confusable words out of both grammars', () => {
    for (const grammar of [EDITOR_GRAMMAR, ANALYSIS_GRAMMAR]) {
      expect(grammar).not.toContain('night')
      expect(grammar).toContain('[unk]')
    }
  })
})
