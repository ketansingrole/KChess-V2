import { describe, expect, it } from 'vitest'
import { goldenFile } from './golden'
import { rules } from '@kchess/rules/engine'
import { Position, type LegalMove, type Role } from '@kchess/rules/position'
import * as voice from '@kchess/rules/voiceCommands'
import * as editor from '@kchess/rules/boardEditor'
import * as coordinates from '@kchess/rules/coordinates'
import * as knight from '@kchess/rules/knight'

/**
 * The voice, board editor, coordinate, knight and material rules are in Rust
 * (`crates/kchess-domain/src/voice.rs`). Each case is generated from a fixed seed and its Rust
 * output is checked against the digest recorded from the TypeScript functions these rules
 * replaced (`golden/voice.json`). `KCHESS_WRITE_GOLDEN=1` rewrites the recording; do that only
 * for an intended behaviour change.
 */
const golden = goldenFile('voice')

/** A seeded generator (mulberry32), as `native-rules.test.ts` uses. */
function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const pick = <T>(random: () => number, items: readonly T[]): T =>
  items[Math.floor(random() * items.length)]!

const PHONETIC = ['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot', 'golf', 'hotel']
const RANK_WORDS = ['one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight']
const PROMOTION_LETTER: Partial<Record<Role, string>> = {
  queen: 'q',
  rook: 'r',
  bishop: 'b',
  knight: 'n',
}

/** A square said three ways: as written, phonetically, or by its letter. */
function spell(square: string, style: number): string {
  if (style === 0) return square
  const rank = RANK_WORDS[Number(square[1]) - 1]
  if (style === 1) return `${PHONETIC[square.charCodeAt(0) - 97]} ${rank}`
  return `${square[0]} ${rank}`
}

/** Positions from the start: fixed FENs first, then random legal games. */
const FIXED_FENS = [
  'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1',
  '4k3/8/8/8/8/8/4K3/R6R w - - 0 1',
  'r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1',
  'r3k2r/pppppppp/8/8/8/8/PPPPPPPP/R3K2R b KQkq - 0 1',
  '7k/P7/8/8/8/8/8/7K w - - 0 1',
  'R7/8/7k/8/8/8/8/R3K3 w - - 0 1',
  '4k3/8/8/3pP3/8/8/8/4K3 w - - 0 1',
  '4k3/8/8/8/8/2b5/8/R3K3 w - - 0 1',
  '4k3/P7/8/8/8/8/8/4K2r w - - 0 1',
  '4k3/8/8/8/8/8/8/4K2R b - - 0 1',
  '8/8/8/8/8/8/8/K6k b - - 3 40',
  'k7/8/8/8/8/8/8/K7 w - - 0 1',
]

function randomPositions(seed: number, games: number): Position[] {
  const random = rng(seed)
  const out: Position[] = []
  for (let game = 0; game < games; game++) {
    let pos = Position.initial()
    const plies = Math.floor(random() * 90)
    for (let ply = 0; ply < plies && !pos.isEnd(); ply++) {
      const moves = pos.legalMoves()
      if (!moves.length) break
      const move = pick(random, moves)
      const uci = `${move.from}${move.to}${move.promotion ? PROMOTION_LETTER[move.promotion] : ''}`
      const played = pos.play(uci)
      if (!played) break
      pos = played.position
      if (random() < 0.1) out.push(pos)
    }
    out.push(pos)
  }
  return out
}

const POSITIONS: Position[] = [
  ...FIXED_FENS.map((fen) => {
    const pos = Position.fromFen(fen)
    if (!pos) throw new Error(`fixed FEN is not a position: ${fen}`)
    return pos
  }),
  ...randomPositions(9101, 40),
]

/** Utterance words: the grammars plus the noise the recognizer produces. */
const VOCABULARY = [
  ...new Set([
    ...voice.MOVE_GRAMMAR,
    ...voice.GAME_GRAMMAR,
    ...voice.EDITOR_GRAMMAR,
    ...voice.ANALYSIS_GRAMMAR,
    '[unk]',
    'the',
    'at',
    'a',
    'night',
    'pawns',
    'kingside',
    'castling',
    'hello',
    'black',
    'white',
  ]),
]

/** A random utterance: vocabulary words with case, punctuation and spacing noise. */
function utterance(random: () => number, words: readonly string[]): string {
  const count = 1 + Math.floor(random() * 6)
  let text = ''
  for (let i = 0; i < count; i++) {
    let word = pick(random, words)
    if (random() < 0.15) word = word.toUpperCase()
    if (random() < 0.1) word += pick(random, ['.', ',', '!', '?'])
    if (i > 0) text += pick(random, [' ', ' ', ' ', '  ', '\t', ' ', ' \n'])
    text += word
  }
  return random() < 0.1 ? ` ${text} ` : text
}

/** The ways one legal move is commonly said. */
function spokenForms(move: LegalMove, random: () => number): string[] {
  const style = () => Math.floor(random() * 3)
  const from = spell(move.from, style())
  const to = spell(move.to, style())
  const forms = [
    `${move.role} ${from} to ${to}`,
    `${from} ${to}`,
    `${move.role} ${to}`,
    `${move.role} takes ${to}`,
    `${to}`,
    `${move.role} from ${from} to ${to}`,
    `${move.role} ${move.from} ${move.to}`,
    `${move.from[0]} ${RANK_WORDS[Number(move.from[1]) - 1]} to ${move.to}`,
  ]
  if (move.promotion) {
    forms.push(`${to} promote to ${move.promotion}`)
    forms.push(`${from} to ${to} equals ${move.promotion}`)
    forms.push(`${to} ${move.promotion}`)
  }
  if (move.san.startsWith('O-O')) {
    forms.push(move.san.startsWith('O-O-O') ? 'castle queen side' : 'castle king side')
    forms.push(move.san.startsWith('O-O-O') ? 'castling queenside' : 'castle kingside')
  }
  return forms
}

const EDGE_SPOKEN_MOVES = [
  'e two to e four',
  'echo two echo four',
  'echo two two echo four',
  'pawn e four',
  'e four',
  'play e4',
  'knight to f three',
  'e five',
  'knight e four',
  'queen e four',
  'e two e four blah',
  'offer draw',
  'e four queen',
  'pawn takes e four',
  'e takes d six',
  'rook d one',
  'rook alpha to d one',
  'h one to d one',
  'castle king side',
  'castling king side',
  'e one to g one',
  'castle queen side',
  'bishop a three',
  'bishop eight three',
  'bishop to eight three',
  'rook eight to a three',
  'rook eight two a three',
  'alpha eight',
  'a eight promote to knight',
  'confirm',
  'cancel',
  'confirm [unk]',
  '[unk] yes play',
  'okay',
  'no',
  '[unk] e four',
  'yes e four',
  'pawn to e four',
  'pawn two e four',
  'pond to e four',
  'pawns e four',
  'pawn two e five',
  'move e four',
  'takes',
  'to',
  '',
  '[unk]',
  'e takes',
  'knight takes e five',
  'two',
  'eight',
  'one two three',
  'e2 e4',
  'E4',
  'Bravo 2',
  'e4.',
]

describe('the Rust voice rules match the recorded TypeScript outputs', () => {
  it('recognizes spoken commands, choices and squares (seed 9001)', () => {
    const random = rng(9001)
    const commandTexts = [
      'take back',
      'Takeback',
      'undo',
      'new game',
      '[unk] resign',
      'flip board',
      'switch sides',
      'knight takes e five',
      'take',
      'back',
      'new',
      'resign now',
      'e four',
      'go back',
      'next',
      'best move',
      'first move',
      'engine on',
      'toggle engine',
      'analysis',
    ]
    const texts = [
      ...commandTexts,
      ...Array.from({ length: 300 }, () => utterance(random, VOCABULARY)),
    ]
    texts.forEach((text, i) => golden.check('spokenCommand', i, rules('spokenCommand', text)))

    const choiceTexts = [
      'one',
      'two',
      'three',
      'eight',
      'Three',
      ' 3 ',
      '3',
      '8',
      '9',
      '0',
      '',
      'play three',
      'Eight ',
      'three.',
    ]
    choiceTexts.forEach((text, i) => golden.check('spokenChoice', i, rules('spokenChoice', text)))

    const squareTexts = [
      ...voice.COORDINATE_GRAMMAR,
      'e nine',
      'e four e five',
      'please play e four',
      'eight three',
      'h eight',
      'a1',
      'Hotel eight',
      'alfa one',
      'queen e four',
      '[unk]',
      '',
    ]
    const more = Array.from({ length: 200 }, () => utterance(random, VOCABULARY))
    ;[...squareTexts, ...more].forEach((text, i) =>
      golden.check('spokenSquare', i, rules('spokenSquare', text)),
    )
  })

  it('reads spoken moves against the positions they are said in (seed 9002)', () => {
    const random = rng(9002)
    const cases: [string, Position][] = []
    for (const pos of POSITIONS) {
      const moves = pos.legalMoves()
      for (let k = 0; k < 4 && moves.length; k++) {
        const move = pick(random, moves)
        for (let f = 0; f < 3; f++) cases.push([pick(random, spokenForms(move, random)), pos])
      }
      cases.push([utterance(random, VOCABULARY), pos])
    }
    for (const text of EDGE_SPOKEN_MOVES) cases.push([text, POSITIONS[0]!])
    const start = Position.initial()
    for (const text of EDGE_SPOKEN_MOVES) cases.push([text, start])
    // Every move the fixed positions allow, with castling and promotion, in each spoken form.
    for (const pos of POSITIONS.slice(0, FIXED_FENS.length)) {
      for (const move of pos.legalMoves()) {
        for (const text of spokenForms(move, random)) cases.push([text, pos])
      }
    }

    cases.forEach(([text, pos], i) =>
      golden.check('spokenMove', i, rules('spokenMove', text, pos.setup)),
    )
  })

  it('reads spoken edits and analysis commands (seed 9003)', () => {
    const random = rng(9003)
    const colors = ['white', 'black', undefined] as const
    const editTexts = [
      'white knight F three',
      'black king on echo eight',
      'queen D one',
      'white pawn eight three',
      'remove E four',
      'clear board',
      'starting position',
      'black to move',
      'analyze',
      'white knight',
      'knight bishop e four',
      'e four',
      'hello',
      'white remove e four',
      'black remove e four',
      'delete a1',
      'empty h8',
      'flip the board',
      'white to play',
      'analysis',
    ]
    const editCases: [string, string | undefined][] = [
      ...editTexts.map((text) => [text, colors[0]] as [string, string | undefined]),
      ...Array.from(
        { length: 300 },
        () => [utterance(random, VOCABULARY), pick(random, colors)] as [string, string | undefined],
      ),
    ]
    editCases.forEach(([text, color], i) =>
      golden.check('spokenEdit', i, rules('spokenEdit', text, color ?? 'white')),
    )

    const analysisTexts = [
      '[unk] back',
      'next',
      'best move',
      'knight f three',
      'go to start',
      'last move',
      'take back',
      'undo',
      'engine off',
      'play best move',
    ]
    const analysisCases = [
      ...analysisTexts,
      ...Array.from({ length: 200 }, () => utterance(random, VOCABULARY)),
    ]
    analysisCases.forEach((text, i) =>
      golden.check('spokenAnalysisCommand', i, rules('spokenAnalysisCommand', text)),
    )
  })

  it('edits boards and writes setup FENs like the editor (seed 9004)', () => {
    const random = rng(9004)
    const roles = ['king', 'queen', 'rook', 'bishop', 'knight', 'pawn'] as const
    const squares = coordinates.ALL_SQUARES
    const boards = [
      ...POSITIONS.map((pos) => pos.fen.split(' ')[0]!),
      editor.EMPTY_BOARD,
      '4k3/8/8/3pP3/8/8/8/4K3',
      '8/8/8/8/8/8/8/8',
      'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR',
      'r3k2r/8/8/8/8/8/8/R3K2R',
    ]
    for (let i = 0; i < 300; i++) {
      const board = pick(random, boards)
      const square = pick(random, squares)
      const piece =
        random() < 0.3
          ? null
          : {
              color: random() < 0.5 ? 'white' : 'black',
              role: pick(random, roles),
            }
      golden.check('withPiece', i, rules('withPiece', board, square, piece))
    }
    // Squares off the board are rejected.
    expect(() => rules('withPiece', editor.EMPTY_BOARD, 'a9', null)).toThrow()

    boards.forEach((board, i) =>
      golden.check('castlingAvailable', i, rules('castlingAvailable', board)),
    )

    let n = 0
    for (const board of boards) {
      for (const turn of ['white', 'black'] as const) {
        golden.check('enPassantSquares', n++, rules('enPassantSquares', board, turn))
      }
    }

    const eps = ['-', '', 'a1', 'd6', 'e3', 'h6', 'c3', 'e6', 'd5']
    for (let i = 0; i < 400; i++) {
      const setup = {
        board: pick(random, boards),
        turn: random() < 0.5 ? 'white' : 'black',
        castling: {
          K: random() < 0.7,
          Q: random() < 0.7,
          k: random() < 0.7,
          q: random() < 0.7,
        },
        ep: pick(random, eps),
      }
      golden.check('setupFen', i, rules('setupFen', setup))
    }
  })

  it('draws squares, plots knight paths and counts material (seed 9005)', () => {
    const random = rng(9005)
    const squares = coordinates.ALL_SQUARES
    const previousDraws = [undefined, ...squares]
    for (let i = 0; i < 300; i++) {
      const previous = pick(random, previousDraws)
      const seed = 20_000 + i
      // The draws the caller would take, in order; the rules pick the first usable one.
      const draws = Array.from({ length: 64 }, rng(seed))
      golden.check('randomSquare', i, rules('randomSquare', previous ?? null, draws))
    }

    const colorSquares = [...squares, '', 'a', 'z9', 'A1', '1a', 'e0', 'i4', 'h8 ']
    colorSquares.forEach((square, i) =>
      golden.check('squareColor', i, rules('squareColor', square)),
    )

    const knightSquares = [...squares, '', 'a', 'z9', 'A1', 'a0', 'a9', 'i1', 'e4x', ' e4', 'e']
    knightSquares.forEach((square, i) => {
      golden.check('knightMoves', i, rules('knightMoves', square))
      golden.check('knightFen', i, rules('knightFen', square))
    })

    const pairs = [
      ...squares.flatMap((from) => squares.map((to) => [from, to] as const)),
      ['a1', 'z9'],
      ['z9', 'z9'],
      ['z9', 'a1'],
      ['a1', 'A1'],
    ]
    pairs.forEach(([from, to], i) =>
      golden.check('knightDistance', i, rules('knightDistance', from, to)),
    )

    const ranges: [number, number][] = [
      [1, 3],
      [3, 4],
      [5, 6],
      [2, 2],
      [0, 0],
      [6, 6],
      [1, 6],
    ]
    for (let i = 0; i < 40; i++) {
      const [min, max] = ranges[i % ranges.length]!
      const seed = 30_000 + i
      // Long-distance pairs are rare: draw until the rules find one, as the caller's loop does.
      let count = 4000
      let found = rules('knightChallenge', min, max, Array.from({ length: count }, rng(seed)))
      while (found === null && count < 1_000_000) {
        count *= 2
        found = rules('knightChallenge', min, max, Array.from({ length: count }, rng(seed)))
      }
      golden.check('knightChallenge', i, found)
    }

    const fens = [
      ...POSITIONS.map((pos) => pos.fen),
      '',
      'PPPP',
      'k',
      'KQkq',
      'QqRrBbNnPp 0 1',
      '8/8/8/8/8/8/8/8 w - - 0 1',
    ]
    fens.forEach((fen, i) => golden.check('materialBalance', i, rules('materialBalance', fen)))
  })

  it('exports the same grammars and constants as the Rust rules', () => {
    // The TypeScript constants stay literal (see voiceCommands.ts), so they are compared directly.
    const constants = {
      COORDINATE_GRAMMAR: voice.COORDINATE_GRAMMAR,
      MOVE_GRAMMAR: voice.MOVE_GRAMMAR,
      GAME_GRAMMAR: voice.GAME_GRAMMAR,
      EDITOR_GRAMMAR: voice.EDITOR_GRAMMAR,
      ANALYSIS_GRAMMAR: voice.ANALYSIS_GRAMMAR,
      FILES: coordinates.FILES,
      RANKS: coordinates.RANKS,
      ALL_SQUARES: coordinates.ALL_SQUARES,
      EMPTY_FEN: coordinates.EMPTY_FEN,
      EMPTY_BOARD: editor.EMPTY_BOARD,
      START_SETUP: editor.START_SETUP,
    }
    Object.entries(constants).forEach(([name, value], i) => {
      const rust = rules(name)
      expect(rust, name).toEqual(JSON.parse(JSON.stringify(value)))
      golden.check('constants', i, rust)
    })
  })
})

describe('the retry loops over the draws', () => {
  it('never repeat the previous square and return a legal knight pair', () => {
    for (let seed = 1; seed < 50; seed++) {
      expect(coordinates.randomSquare('a1', rng(seed))).not.toBe('a1')
      const found = knight.knightChallenge(1, 1, rng(seed))
      expect(found.best).toBe(1)
      expect(knight.knightMoves(found.from)).toContain(found.to)
    }
  })
})
