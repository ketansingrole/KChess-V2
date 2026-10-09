import type { SelectItem } from '@nuxt/ui'

/** Puzzle themes as Lichess names them (the `angle` of a puzzle request), grouped for the picker. */

export interface PuzzleTheme {
  key: string
  name: string
  hint: string
}

export interface PuzzleThemeGroup {
  label: string
  themes: PuzzleTheme[]
}

export const PUZZLE_THEME_GROUPS: PuzzleThemeGroup[] = [
  {
    label: 'Mate patterns',
    themes: [
      { key: 'mateIn1', name: 'Mate in 1', hint: 'Deliver checkmate in one move.' },
      { key: 'mateIn2', name: 'Mate in 2', hint: 'Deliver checkmate in two moves.' },
      { key: 'mateIn3', name: 'Mate in 3', hint: 'Deliver checkmate in three moves.' },
      { key: 'mateIn4', name: 'Mate in 4', hint: 'Deliver checkmate in four moves.' },
      { key: 'mateIn5', name: 'Mate in 5 or more', hint: 'A long forced mate.' },
      {
        key: 'backRankMate',
        name: 'Back rank mate',
        hint: 'Mate a king trapped behind its own pawns.',
      },
      {
        key: 'smotheredMate',
        name: 'Smothered mate',
        hint: 'A knight mates a king boxed in by its own pieces.',
      },
      {
        key: 'anastasiaMate',
        name: 'Anastasia’s mate',
        hint: 'Knight and rook trap a king against the edge and its own pawn.',
      },
      {
        key: 'arabianMate',
        name: 'Arabian mate',
        hint: 'Knight and rook mate a king in the corner.',
      },
      {
        key: 'bodensMate',
        name: 'Boden’s mate',
        hint: 'Two bishops on crossing diagonals deliver mate.',
      },
      {
        key: 'doubleBishopMate',
        name: 'Double bishop mate',
        hint: 'Two bishops on adjacent diagonals deliver mate.',
      },
      {
        key: 'dovetailMate',
        name: 'Dovetail mate',
        hint: 'A queen mates a king whose escape squares are blocked by its own pieces.',
      },
      { key: 'hookMate', name: 'Hook mate', hint: 'Rook, knight and a pawn combine to mate.' },
      {
        key: 'killBoxMate',
        name: 'Kill box mate',
        hint: 'A rook, protected by the queen, mates in a tight box.',
      },
      {
        key: 'vukovicMate',
        name: 'Vukovic mate',
        hint: 'Rook and knight, with a supporting pawn, mate the king.',
      },
    ],
  },
  {
    label: 'Tactical motifs',
    themes: [
      { key: 'fork', name: 'Fork', hint: 'One piece attacks two or more at once.' },
      {
        key: 'pin',
        name: 'Pin',
        hint: 'A piece cannot move without exposing something more valuable.',
      },
      {
        key: 'skewer',
        name: 'Skewer',
        hint: 'A valuable piece is forced to move, exposing one behind it.',
      },
      {
        key: 'discoveredAttack',
        name: 'Discovered attack',
        hint: 'Moving a piece uncovers an attack from another.',
      },
      { key: 'doubleCheck', name: 'Double check', hint: 'Two pieces give check at once.' },
      { key: 'hangingPiece', name: 'Hanging piece', hint: 'An undefended piece can be taken.' },
      { key: 'trappedPiece', name: 'Trapped piece', hint: 'A piece has no safe square to go to.' },
      { key: 'exposedKing', name: 'Exposed king', hint: 'A king with little cover is in danger.' },
      {
        key: 'capturingDefender',
        name: 'Capture the defender',
        hint: 'Remove the piece that guards another.',
      },
      { key: 'sacrifice', name: 'Sacrifice', hint: 'Give up material for a bigger gain.' },
      {
        key: 'attackingF2F7',
        name: 'Attacking f2 or f7',
        hint: 'Target the weakest squares next to the king.',
      },
      { key: 'kingsideAttack', name: 'Kingside attack', hint: 'Attack a king that castled short.' },
      {
        key: 'queensideAttack',
        name: 'Queenside attack',
        hint: 'Attack a king that castled long.',
      },
      { key: 'advancedPawn', name: 'Advanced pawn', hint: 'A far-advanced pawn decides the game.' },
    ],
  },
  {
    label: 'Advanced tactics',
    themes: [
      { key: 'attraction', name: 'Attraction', hint: 'Lure a piece onto a bad square.' },
      { key: 'clearance', name: 'Clearance', hint: 'Move a piece out of the way with tempo.' },
      { key: 'defensiveMove', name: 'Defensive move', hint: 'Find the only move that holds.' },
      { key: 'deflection', name: 'Deflection', hint: 'Force a defender away from its duty.' },
      {
        key: 'interference',
        name: 'Interference',
        hint: 'Block the line between two enemy pieces.',
      },
      {
        key: 'intermezzo',
        name: 'Intermezzo',
        hint: 'Insert an in-between move before the expected reply.',
      },
      {
        key: 'quietMove',
        name: 'Quiet move',
        hint: 'A move that neither checks nor captures, yet wins.',
      },
      { key: 'xRayAttack', name: 'X-ray attack', hint: 'A piece attacks through another.' },
      {
        key: 'zugzwang',
        name: 'Zugzwang',
        hint: 'Every move the opponent has makes things worse.',
      },
    ],
  },
  {
    label: 'Endgames',
    themes: [
      { key: 'endgame', name: 'Endgame', hint: 'A few pieces are left.' },
      { key: 'pawnEndgame', name: 'Pawn endgame', hint: 'Only kings and pawns.' },
      { key: 'rookEndgame', name: 'Rook endgame', hint: 'Rooks and pawns.' },
      { key: 'bishopEndgame', name: 'Bishop endgame', hint: 'Bishops and pawns.' },
      { key: 'knightEndgame', name: 'Knight endgame', hint: 'Knights and pawns.' },
      { key: 'queenEndgame', name: 'Queen endgame', hint: 'Queens and pawns.' },
      { key: 'queenRookEndgame', name: 'Queen and rook endgame', hint: 'Queens, rooks and pawns.' },
    ],
  },
  {
    label: 'Special moves and goals',
    themes: [
      { key: 'promotion', name: 'Promotion', hint: 'Push a pawn to the last rank.' },
      {
        key: 'underPromotion',
        name: 'Underpromotion',
        hint: 'Promote to a knight, bishop or rook.',
      },
      { key: 'castling', name: 'Castling', hint: 'Castling is the key move.' },
      { key: 'enPassant', name: 'En passant', hint: 'Capture a pawn as it passes.' },
      { key: 'advantage', name: 'Advantage', hint: 'Turn a good position into a winning one.' },
      { key: 'crushing', name: 'Crushing', hint: 'Punish a blunder.' },
      { key: 'equality', name: 'Equality', hint: 'Save a worse position.' },
    ],
  },
  {
    label: 'Phase and length',
    themes: [
      { key: 'opening', name: 'Opening', hint: 'From the first moves of a game.' },
      { key: 'middlegame', name: 'Middlegame', hint: 'From the middle of a game.' },
      { key: 'oneMove', name: 'One-move puzzle', hint: 'Just one move to find.' },
      { key: 'short', name: 'Short', hint: 'Two moves to win.' },
      { key: 'long', name: 'Long', hint: 'Three moves to win.' },
      { key: 'veryLong', name: 'Very long', hint: 'Four moves or more.' },
    ],
  },
]

const ALL_THEMES = PUZZLE_THEME_GROUPS.flatMap((group) => group.themes)
const BY_KEY = new Map(ALL_THEMES.map((theme) => [theme.key, theme]))

/** The name Lichess gives a theme key; unknown keys are split into words. */
export function themeName(key: string): string {
  if (key === 'mix') return 'Healthy mix'
  return (
    BY_KEY.get(key)?.name ?? key.replace(/([A-Z])/g, ' $1').replace(/^./, (c) => c.toUpperCase())
  )
}

export const themeHint = (key: string): string | undefined => BY_KEY.get(key)?.hint

/** Options for a select: the mix, then every theme under its group. */
export function themeItems(): SelectItem[] {
  return [
    { label: 'Healthy mix', value: 'mix' },
    ...PUZZLE_THEME_GROUPS.flatMap((group): SelectItem[] => [
      { type: 'separator' },
      { type: 'label', label: group.label },
      ...group.themes.map((theme) => ({ label: theme.name, value: theme.key })),
    ]),
  ]
}
