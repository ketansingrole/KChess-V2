import type { SelectItem } from '@nuxt/ui'
import { PUZZLE_THEME_GROUPS } from '@kchess/rules/puzzleThemes'

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

/** A puzzle's first few themes by name, leaving out its length (short, long …). */
export function puzzleThemeText(themes: readonly string[]): string {
  return themes
    .filter((theme) => !['short', 'long', 'veryLong', 'oneMove'].includes(theme))
    .slice(0, 3)
    .map(themeName)
    .join(', ')
}
