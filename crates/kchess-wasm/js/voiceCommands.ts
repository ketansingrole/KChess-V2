import { rules } from './engine.ts'
import type { Position, Role } from './position.ts'
import { ALL_SQUARES, type Square } from './coordinates'

/*
 * Spoken commands, moves and edits for Vosk's small English model. The matching is in Rust
 * (`crates/kchess-domain/src/voice.rs`). The grammar lists stay here as constants: the renderer
 * reads them when this module loads, before the rules are bound, and
 * `tests/core/native-voice.test.ts` pins them to the Rust lists.
 */

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
  return rules<GameCommand | null>('spokenCommand', text) ?? undefined
}

export function spokenChoice(text: string): number | undefined {
  return rules<number | null>('spokenChoice', text) ?? undefined
}

export function spokenSquare(text: string): Square | undefined {
  return rules<Square | null>('spokenSquare', text) ?? undefined
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
  return rules<VoiceMoveResult>('spokenMove', text, pos.setup)
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
  return rules<EditorVoiceAction | null>('spokenEdit', text, color) ?? undefined
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
  return rules<AnalysisCommand | null>('spokenAnalysisCommand', text) ?? undefined
}
