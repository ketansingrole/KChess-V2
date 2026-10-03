import type { Position } from 'chessops/chess'
import { makeSan } from 'chessops/san'
import { makeSquare, makeUci } from 'chessops/util'
import type { Role } from 'chessops/types'
import { ALL_SQUARES, type Square } from './coordinates'

const fileWords: Record<string, string> = {
  a: 'a',
  alpha: 'a',
  alfa: 'a',
  ay: 'a',
  b: 'b',
  bravo: 'b',
  bee: 'b',
  c: 'c',
  charlie: 'c',
  see: 'c',
  d: 'd',
  delta: 'd',
  dee: 'd',
  e: 'e',
  echo: 'e',
  f: 'f',
  foxtrot: 'f',
  g: 'g',
  golf: 'g',
  gee: 'g',
  h: 'h',
  hotel: 'h',
  aitch: 'h',
}
const rankWords: Record<string, string> = {
  one: '1',
  two: '2',
  three: '3',
  four: '4',
  five: '5',
  six: '6',
  seven: '7',
  eight: '8',
}
const roles: Record<string, Role> = {
  pawn: 'pawn',
  pawns: 'pawn',
  // Where “pawn” is said “paan”/“pon” (most accents outside North America), the small model's
  // nearest word is “pond”, not “pawn”.
  pond: 'pawn',
  knight: 'knight',
  night: 'knight',
  bishop: 'bishop',
  rook: 'rook',
  queen: 'queen',
  king: 'king',
}
const phonetics = ['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot', 'golf', 'hotel']

export const COORDINATE_GRAMMAR = [
  ...ALL_SQUARES.flatMap((square) => [
    `${square[0]} ${Object.keys(rankWords)[Number(square[1]) - 1]}`,
    `${phonetics[square.charCodeAt(0) - 97]} ${Object.keys(rankWords)[Number(square[1]) - 1]}`,
  ]),
  '[unk]',
]
const CONFIRM_WORDS = ['confirm', 'yes', 'yeah', 'okay']
const CANCEL_WORDS = ['cancel', 'no']
/** Filler that may accompany a confirmation: “yes play”, “confirm move”. */
const CONFIRM_FILLER = ['play', 'move']

export const MOVE_GRAMMAR = [
  ...COORDINATE_GRAMMAR,
  // “night” stays in the parser but not the grammar: as a homophone it halves “knight”'s confidence.
  // “pawns” is left out for the same reason (it adds nothing “pawn” doesn't catch).
  ...Object.keys(roles).filter((word) => word !== 'night' && word !== 'pawns'),
  'to',
  'from',
  'takes',
  'captures',
  'move',
  'play',
  // Only words in the small model's vocabulary; "kingside"/"castling" are not, and would be dropped.
  'castle',
  'king side',
  'queen side',
  'promote',
  'promotion',
  'equals',
  ...CONFIRM_WORDS,
  ...CANCEL_WORDS,
  ...Object.keys(rankWords),
]

/** Game controls, recognized only as the whole phrase so “knight takes …” never reads as one. */
export type GameCommand = 'takeback' | 'newGame' | 'resign' | 'flip'
const GAME_COMMANDS: Record<string, GameCommand> = {
  'take back': 'takeback',
  undo: 'takeback',
  'new game': 'newGame',
  resign: 'resign',
  'flip board': 'flip',
  'switch sides': 'flip',
}
/** Moves plus the game controls, for Play with Computer. */
export const GAME_GRAMMAR = [...MOVE_GRAMMAR, ...Object.keys(GAME_COMMANDS)]

export function spokenCommand(text: string): GameCommand | undefined {
  const phrase = text
    .toLowerCase()
    .replace(/[.,!?]/g, '')
    .split(/\s+/)
    .filter((word) => word && word !== '[unk]')
    .join(' ')
  return GAME_COMMANDS[phrase === 'takeback' ? 'take back' : phrase]
}

export function spokenChoice(text: string): number | undefined {
  const normalized = text.trim().toLowerCase()
  const rank = rankWords[normalized] ?? normalized
  return /^[1-8]$/.test(rank) ? Number(rank) : undefined
}

/** Normalize exact words only: unrelated speech must never become a move by fuzzy matching. */
function tokens(text: string): string[] {
  const words = text
    .toLowerCase()
    .replace(/[.,!?]/g, '')
    .split(/\s+/)
    // Vosk marks noise it could not match to the grammar as [unk]; it carries no meaning.
    .filter((word) => word && word !== '[unk]')
  const result: string[] = []
  for (let i = 0; i < words.length; i++) {
    const word = words[i]!
    // The letter “a” said “ay” is often heard as “eight”: “bishop a three” → “bishop eight three”.
    // Rank-then-rank is never a square, so read it as file a — unless a square follows, as in
    // “rook eight to a three” (the “to” heard as “two”), where the eight is a source rank.
    const file =
      word === 'eight' && rankWords[words[i + 1]!] && !fileWords[words[i + 2]!]
        ? 'a'
        : fileWords[word]
    const rank =
      rankWords[words[i + 1]!] ?? (/^[1-8]$/.test(words[i + 1] ?? '') ? words[i + 1] : undefined)
    if (file && rank) {
      result.push(`${file}${rank}`)
      i++
    } else result.push(file ?? rankWords[word] ?? word)
  }
  return result
}

export function spokenSquare(text: string): Square | undefined {
  const words = tokens(text)
  return words.length === 1 && /^[a-h][1-8]$/.test(words[0]!) ? (words[0] as Square) : undefined
}

export interface VoiceMoveChoice {
  uci: string
  san: string
}
export type VoiceMoveResult =
  | { kind: 'move'; choices: VoiceMoveChoice[] }
  | { kind: 'confirm' }
  | { kind: 'cancel' }
  | { kind: 'invalid' }

/** Resolve only against the actual playable position, including underpromotion and castling. */
export function spokenMove(text: string, pos: Position): VoiceMoveResult {
  const words = tokens(text)
  if (
    words.some((word) => CONFIRM_WORDS.includes(word)) &&
    words.every((word) => CONFIRM_WORDS.includes(word) || CONFIRM_FILLER.includes(word))
  )
    return { kind: 'confirm' }
  if (words.length && words.every((word) => CANCEL_WORDS.includes(word))) return { kind: 'cancel' }
  if (words[0] === 'move' || words[0] === 'play') words.shift()
  const castle = words.join(' ').replace('castling', 'castle')
  let castleSide: 'king' | 'queen' | undefined
  if (['castle kingside', 'castle king side'].includes(castle)) castleSide = 'king'
  else if (['castle queenside', 'castle queen side'].includes(castle)) castleSide = 'queen'

  let promotion: Role | undefined
  const last = roles[words.at(-1) ?? '']
  if (last && ['queen', 'rook', 'bishop', 'knight'].includes(last)) {
    promotion = last
    words.pop()
    if (words.at(-1) === 'to') words.pop()
    if (['promote', 'promotion', 'equals'].includes(words.at(-1) ?? '')) words.pop()
  }
  const dest = words.pop()
  const capture = words.includes('takes') || words.includes('captures')
  let role: Role | undefined
  if (roles[words[0] ?? '']) role = roles[words.shift()!]!
  if (words[0] === 'from') words.shift()
  // Recognizers can hear the separator “to” as “two”. Only reinterpret it
  // between two complete coordinates, where it cannot be a source rank.
  if (words.length === 2 && /^[a-h][1-8]$/.test(words[0]!) && words[1] === '2') words.pop()
  if (['to', 'takes', 'captures'].includes(words.at(-1) ?? '')) words.pop()
  const source = words[0]
  if (
    !castleSide &&
    (!dest ||
      !/^[a-h][1-8]$/.test(dest) ||
      words.length > 1 ||
      (source && !/^(?:[a-h][1-8]|[a-h]|[1-8])$/.test(source)))
  )
    return { kind: 'invalid' }
  // A square alone means a pawn move; piece names and explicit origins are also supported.
  if (!castleSide && !role && !source) role = 'pawn'
  const choices = matchingMoves(pos, { castleSide, dest, role, source, capture, promotion })
  // “Pawn to e5” is often heard as “pawn two e five”. A lone rank 2 that fits no move was that “to”.
  if (!choices.length && source === '2' && role)
    choices.push(...matchingMoves(pos, { castleSide, dest, role, capture, promotion }))
  return choices.length ? { kind: 'move', choices } : { kind: 'invalid' }
}

function matchingMoves(
  pos: Position,
  spoken: {
    castleSide?: 'king' | 'queen'
    dest?: string
    role?: Role
    source?: string
    capture: boolean
    promotion?: Role
  },
): VoiceMoveChoice[] {
  const { castleSide, dest, role, source, capture, promotion } = spoken
  const choices: VoiceMoveChoice[] = []
  for (const [from, dests] of pos.allDests()) {
    const piece = pos.board.get(from)!
    for (const to of dests) {
      const promotes = piece.role === 'pawn' && (to >> 3 === 0 || to >> 3 === 7)
      const promotions: (Role | undefined)[] = promotes
        ? ['queen', 'rook', 'bishop', 'knight']
        : [undefined]
      for (const promoted of promotions) {
        const move = { from, to, ...(promoted ? { promotion: promoted } : {}) }
        if (!pos.isLegal(move)) continue
        const san = makeSan(pos, move)
        if (castleSide) {
          if (san.replace(/[+#]$/, '') !== (castleSide === 'king' ? 'O-O' : 'O-O-O')) continue
        } else {
          const castleDest = san.startsWith('O-O')
            ? `${san.startsWith('O-O-O') ? 'c' : 'g'}${pos.turn === 'white' ? '1' : '8'}`
            : undefined
          if ((makeSquare(to) !== dest && castleDest !== dest) || (role && piece.role !== role))
            continue
          if (source && !makeSquare(from).includes(source)) continue
          if (capture && !san.includes('x')) continue
          if (promotion && promoted !== promotion) continue
        }
        // chessops encodes castling as king-takes-rook (e1h1); engines expect standard UCI (e1g1).
        const uci = san.startsWith('O-O')
          ? `${makeSquare(from)}${san.startsWith('O-O-O') ? 'c' : 'g'}${makeSquare(from)[1]}`
          : makeUci(move)
        if (!choices.some((choice) => choice.uci === uci)) choices.push({ uci, san })
      }
    }
  }
  return choices
}

/* ── Board editor ─────────────────────────────────────────────────────── */

export type PieceColor = 'white' | 'black'
export type EditorVoiceAction =
  | { kind: 'place'; square: Square; color: PieceColor; role: Role }
  | { kind: 'remove'; square: Square }
  | { kind: 'clear' }
  | { kind: 'start' }
  | { kind: 'turn'; color: PieceColor }
  | { kind: 'flip' }
  | { kind: 'analyze' }

const REMOVE_WORDS = ['remove', 'clear', 'delete', 'empty']
const EDITOR_PHRASES: Record<string, EditorVoiceAction> = {
  'clear board': { kind: 'clear' },
  'empty board': { kind: 'clear' },
  'clear the board': { kind: 'clear' },
  'starting position': { kind: 'start' },
  'start position': { kind: 'start' },
  'reset board': { kind: 'start' },
  'reset the board': { kind: 'start' },
  'white to move': { kind: 'turn', color: 'white' },
  'white to play': { kind: 'turn', color: 'white' },
  'black to move': { kind: 'turn', color: 'black' },
  'black to play': { kind: 'turn', color: 'black' },
  'flip board': { kind: 'flip' },
  'flip the board': { kind: 'flip' },
  analyze: { kind: 'analyze' },
  analysis: { kind: 'analyze' },
}

export const EDITOR_GRAMMAR = [
  ...COORDINATE_GRAMMAR,
  ...Object.keys(roles).filter((word) => word !== 'night' && word !== 'pawns'),
  'white',
  'black',
  'on',
  'to',
  ...REMOVE_WORDS,
  ...Object.keys(EDITOR_PHRASES),
]

/**
 * “White knight F three”, “black king on E eight”, “remove E four”, “clear board”,
 * “starting position”, “black to move”. A piece without a colour takes `color`, the last one used.
 */
export function spokenEdit(
  text: string,
  color: PieceColor = 'white',
): EditorVoiceAction | undefined {
  const phrase = text
    .toLowerCase()
    .replace(/[.,!?]/g, '')
    .split(/\s+/)
    .filter((word) => word && word !== '[unk]')
    .join(' ')
  const fixed = EDITOR_PHRASES[phrase]
  if (fixed) return fixed
  const words = tokens(text).filter((word) => !['on', 'to', 'at', 'the'].includes(word))
  const square = words.at(-1)
  if (!square || !/^[a-h][1-8]$/.test(square)) return undefined
  const rest = words.slice(0, -1)
  if (rest.length === 1 && REMOVE_WORDS.includes(rest[0]!))
    return { kind: 'remove', square: square as Square }
  let spokenColor: PieceColor | undefined
  if (rest[0] === 'white' || rest[0] === 'black') spokenColor = rest.shift() as PieceColor
  const role = rest.length === 1 ? roles[rest[0]!] : undefined
  return role
    ? { kind: 'place', square: square as Square, color: spokenColor ?? color, role }
    : undefined
}

/* ── Analysis board ───────────────────────────────────────────────────── */

export type AnalysisCommand = 'back' | 'forward' | 'start' | 'end' | 'best' | 'flip' | 'engine'
const ANALYSIS_COMMANDS: Record<string, AnalysisCommand> = {
  back: 'back',
  'go back': 'back',
  previous: 'back',
  undo: 'back',
  'take back': 'back',
  forward: 'forward',
  next: 'forward',
  'go forward': 'forward',
  'first move': 'start',
  'go to start': 'start',
  'last move': 'end',
  'go to end': 'end',
  'best move': 'best',
  'play best move': 'best',
  'flip board': 'flip',
  'toggle engine': 'engine',
  'engine on': 'engine',
  'engine off': 'engine',
}
/** Moves, plus stepping through the tree and engine controls. */
export const ANALYSIS_GRAMMAR = [...MOVE_GRAMMAR, ...Object.keys(ANALYSIS_COMMANDS)]

export function spokenAnalysisCommand(text: string): AnalysisCommand | undefined {
  const phrase = text
    .toLowerCase()
    .replace(/[.,!?]/g, '')
    .split(/\s+/)
    .filter((word) => word && word !== '[unk]')
    .join(' ')
  return ANALYSIS_COMMANDS[phrase === 'takeback' ? 'take back' : phrase]
}
