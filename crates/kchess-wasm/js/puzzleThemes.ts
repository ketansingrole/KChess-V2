import groups from '../../kchess-domain/data/puzzle-themes.json' with { type: 'json' }

/**
 * Puzzle themes as Lichess names them (the `angle` of a puzzle request), grouped for the picker;
 * shared by every frontend (`data/puzzle-themes.json`).
 */
export interface PuzzleTheme {
  key: string
  name: string
  hint: string
}

export interface PuzzleThemeGroup {
  label: string
  themes: PuzzleTheme[]
}

export const PUZZLE_THEME_GROUPS = groups as readonly PuzzleThemeGroup[]
