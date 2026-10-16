/** Each endgame course's card icon (named here so the offline icon bundle includes it). */
const ICONS: Record<string, string> = {
  'piece-checkmates-1': 'i-lucide-chess-queen',
  'checkmate-patterns-1': 'i-lucide-target',
  'checkmate-patterns-2': 'i-lucide-crosshair',
  'checkmate-patterns-3': 'i-lucide-focus',
  'checkmate-patterns-4': 'i-lucide-puzzle',
  'piece-checkmates-2': 'i-lucide-chess-king',
  'knight-bishop-mate': 'i-lucide-chess-knight',
  'key-squares': 'i-lucide-key-round',
  opposition: 'i-lucide-arrow-left-right',
  'rook-pawn-vs-queen': 'i-lucide-chess-pawn',
  'rook-pawn-vs-rook': 'i-lucide-chess-rook',
  'basic-rook-endgames': 'i-lucide-castle',
  'intermediate-rook-endings': 'i-lucide-tower-control',
  'practical-rook-endings': 'i-lucide-brick-wall',
}

export const courseIcon = (id: string): string => ICONS[id] ?? 'i-lucide-graduation-cap'
