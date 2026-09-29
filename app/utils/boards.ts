/** Lichess 2D board themes, matching `ui/lib/css/theme/board/_boards.scss` in lila. */
export const boardThemes = [
  { id: 'brown', name: 'Brown' },
  { id: 'blue', name: 'Blue' },
  { id: 'blue2', name: 'Blue 2' },
  { id: 'green', name: 'Green' },
  { id: 'green-plastic', name: 'Green plastic' },
  { id: 'purple', name: 'Purple' },
  { id: 'purple-diag', name: 'Purple diagonal' },
  { id: 'ic', name: 'IC' },
  { id: 'wood', name: 'Wood' },
  { id: 'maple', name: 'Maple' },
  { id: 'grey', name: 'Grey' },
  { id: 'marble', name: 'Marble' },
  { id: 'leather', name: 'Leather' },
] as const

export type BoardTheme = (typeof boardThemes)[number]['id']
