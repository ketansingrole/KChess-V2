import { describe, expect, it } from 'vitest'
import {
  spokenMove,
  spokenSquare,
  spokenChoice,
  spokenCommand,
  COORDINATE_GRAMMAR,
  GAME_GRAMMAR,
  MOVE_GRAMMAR,
} from '@kchess/rules/voiceCommands'
import { positionAfter, positionFromFen } from '@kchess/rules/chess'

describe('spoken square names', () => {
  it.each([
    ['E four', 'e4'],
    ['echo four', 'e4'],
    ['Bravo 2', 'b2'],
    ['a1', 'a1'],
    ['Hotel eight', 'h8'],
    ['Charlie three.', 'c3'],
    ['alfa one', 'a1'],
  ])('recognizes %s', (text, square) => expect(spokenSquare(text)).toBe(square))
  it.each(['e nine', 'e four e five', 'please play e four', '[unk]', '', 'queen e four'])(
    'rejects %s',
    (text) => {
      expect(spokenSquare(text)).toBeUndefined()
    },
  )
  it('has equal coverage of all 64 answers and an unknown-speech alternative', () => {
    expect(new Set(COORDINATE_GRAMMAR.map(spokenSquare).filter(Boolean)).size).toBe(64)
    expect(COORDINATE_GRAMMAR).toContain('[unk]')
  })
})

describe('spoken legal moves', () => {
  it.each([
    'e two to e four',
    'echo two echo four',
    'echo two two echo four',
    'pawn e four',
    'e four',
    'play e4',
  ])('resolves %s', (text) => {
    expect(spokenMove(text, positionAfter([]))).toEqual({
      kind: 'move',
      choices: [{ uci: 'e2e4', san: 'e4' }],
    })
  })
  it('supports natural piece moves', () => {
    expect(spokenMove('knight to f three', positionAfter([]))).toEqual({
      kind: 'move',
      choices: [{ uci: 'g1f3', san: 'Nf3' }],
    })
  })
  it.each([
    'e five',
    'knight e four',
    'queen e four',
    'e two e four blah',
    'offer draw',
    'e four queen',
  ])('rejects %s', (text) => {
    expect(spokenMove(text, positionAfter([]))).toEqual({ kind: 'invalid' })
  })
  it('checks the actual side to move', () => {
    expect(spokenMove('e two e four', positionAfter(['d2d4']))).toEqual({ kind: 'invalid' })
  })
  it('requires a capture when takes is spoken and recognizes en passant', () => {
    expect(spokenMove('pawn takes e four', positionAfter([]))).toEqual({ kind: 'invalid' })
    const pos = positionAfter(['e2e4', 'a7a6', 'e4e5', 'd7d5'])
    expect(spokenMove('e takes d six', pos)).toEqual({
      kind: 'move',
      choices: [{ uci: 'e5d6', san: 'exd6' }],
    })
  })
  it('returns ambiguous moves and accepts file/rank/origin clarification', () => {
    const pos = positionFromFen('4k3/8/8/8/8/8/4K3/R6R w - - 0 1')!
    const parsed = spokenMove('rook d one', pos)
    expect(parsed.kind === 'move' && parsed.choices.map((choice) => choice.uci)).toEqual([
      'a1d1',
      'h1d1',
    ])
    expect(spokenMove('rook alpha to d one', pos)).toEqual({
      kind: 'move',
      choices: [{ uci: 'a1d1', san: 'Rad1' }],
    })
    expect(spokenMove('h one to d one', pos)).toEqual({
      kind: 'move',
      choices: [{ uci: 'h1d1', san: 'Rhd1' }],
    })
  })
  it('sends castling to the engine in standard UCI, which Stockfish accepts', () => {
    const moves = ['e2e4', 'e7e5', 'g1f3', 'b8c6', 'f1c4', 'g8f6']
    const result = spokenMove('castle king side', positionAfter(moves))
    expect(result).toEqual({ kind: 'move', choices: [{ uci: 'e1g1', san: 'O-O' }] })
    expect(positionAfter([...moves, 'e1g1']).turn).toBe('black')
  })
  it('handles castling names and king destination coordinates', () => {
    const pos = positionFromFen('r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1')!
    expect(spokenMove('castle king side', pos)).toEqual({
      kind: 'move',
      choices: [{ uci: 'e1g1', san: 'O-O' }],
    })
    expect(spokenMove('e one to g one', pos)).toEqual(spokenMove('castle kingside', pos))
    expect(spokenMove('castle queen side', pos)).toEqual({
      kind: 'move',
      choices: [{ uci: 'e1c1', san: 'O-O-O' }],
    })
  })
  it('reads “eight” before a rank as the letter a, which it is often heard for', () => {
    const pos = positionAfter(['b2b3', 'e7e5'])
    for (const text of ['bishop a three', 'bishop eight three', 'bishop to eight three'])
      expect(spokenMove(text, pos)).toEqual({
        kind: 'move',
        choices: [{ uci: 'c1a3', san: 'Ba3' }],
      })
    expect(spokenSquare('eight three')).toBe('a3')
    expect(spokenSquare('h eight')).toBe('h8')
    // A real source rank before a square stays a rank.
    const rooks = positionFromFen('R7/8/7k/8/8/8/8/R3K3 w - - 0 1')!
    expect(spokenMove('rook eight to a three', rooks)).toEqual({
      kind: 'move',
      choices: [{ uci: 'a8a3', san: 'R8a3' }],
    })
    // …and never becomes a2 when “to” is heard as “two”.
    expect(spokenMove('rook eight two a three', rooks)).toEqual({ kind: 'invalid' })
  })
  it('requires a promotion choice rather than silently choosing a queen', () => {
    const pos = positionFromFen('7k/P7/8/8/8/8/8/7K w - - 0 1')!
    const parsed = spokenMove('alpha eight', pos)
    expect(parsed.kind === 'move' && parsed.choices.length).toBe(4)
    expect(spokenMove('a eight promote to knight', pos)).toEqual({
      kind: 'move',
      choices: [{ uci: 'a7a8n', san: 'a8=N' }],
    })
  })
  it('recognizes confirmations and numbered choices exactly', () => {
    expect(spokenMove('confirm', positionAfter([]))).toEqual({ kind: 'confirm' })
    expect(spokenMove('cancel', positionAfter([]))).toEqual({ kind: 'cancel' })
    expect(spokenChoice('three')).toBe(3)
    expect(spokenChoice('play three')).toBeUndefined()
  })
  it('ignores noise tokens and accepts natural confirmations', () => {
    const pos = positionAfter([])
    expect(spokenMove('confirm [unk]', pos)).toEqual({ kind: 'confirm' })
    expect(spokenMove('[unk] yes play', pos)).toEqual({ kind: 'confirm' })
    expect(spokenMove('okay', pos)).toEqual({ kind: 'confirm' })
    expect(spokenMove('no', pos)).toEqual({ kind: 'cancel' })
    expect(spokenMove('[unk] e four', pos)).toEqual({
      kind: 'move',
      choices: [{ uci: 'e2e4', san: 'e4' }],
    })
    // A confirmation word mixed into a move is not a confirmation.
    expect(spokenMove('yes e four', pos).kind).not.toBe('confirm')
  })
  it('understands the ways “pawn to …” is commonly heard', () => {
    const start = positionAfter([])
    for (const text of ['pawn to e four', 'pawn two e four', 'pond to e four', 'pawns e four'])
      expect(spokenMove(text, start)).toEqual({
        kind: 'move',
        choices: [{ uci: 'e2e4', san: 'e4' }],
      })
    // As Black, “to” heard as “two” is not a source rank: no black pawn stands on rank 2.
    expect(spokenMove('pawn two e five', positionAfter(['e2e4']))).toEqual({
      kind: 'move',
      choices: [{ uci: 'e7e5', san: 'e5' }],
    })
  })

  it('only offers words the small English model knows', () => {
    for (const word of ['kingside', 'queenside', 'castling', 'night'])
      expect(MOVE_GRAMMAR).not.toContain(word)
  })
})

describe('spoken game commands', () => {
  it.each([
    ['take back', 'takeback'],
    ['Takeback', 'takeback'],
    ['undo', 'takeback'],
    ['new game', 'newGame'],
    ['[unk] resign', 'resign'],
    ['flip board', 'flip'],
    ['switch sides', 'flip'],
  ])('recognizes %s', (text, command) => expect(spokenCommand(text)).toBe(command))
  it.each(['knight takes e five', 'take', 'back', 'new', 'resign now', 'e four'])(
    'only matches the whole phrase, not %s',
    (text) => expect(spokenCommand(text)).toBeUndefined(),
  )
  it('offers the command phrases in the game grammar', () => {
    for (const phrase of ['take back', 'undo', 'new game', 'resign', 'flip board', 'switch sides'])
      expect(GAME_GRAMMAR).toContain(phrase)
  })
})
