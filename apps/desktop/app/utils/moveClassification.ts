import type { MoveClassification } from '@kchess/rules/review'

/** Shared by the board, move list and review feedback. */
export const MOVE_CLASSIFICATIONS: Record<
  MoveClassification,
  { label: string; glyph: string; color: string }
> = {
  brilliant: { label: 'Brilliant', glyph: '!!', color: '#168c9c' },
  best: { label: 'Best', glyph: '★', color: '#538425' },
  excellent: { label: 'Excellent', glyph: '✓', color: '#658443' },
  good: { label: 'Good', glyph: '•', color: '#697f51' },
  inaccuracy: { label: 'Inaccuracy', glyph: '?!', color: 'var(--judgment-inaccuracy)' },
  mistake: { label: 'Mistake', glyph: '?', color: 'var(--judgment-mistake)' },
  blunder: { label: 'Blunder', glyph: '??', color: 'var(--judgment-blunder)' },
}
