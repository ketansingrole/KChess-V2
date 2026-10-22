import type { AppTheme, ThemePalette } from '@kchess/contracts/types'
import { rules } from './engine.ts'
import presets from '../../kchess-domain/data/themes.json' with { type: 'json' }

/** The preset app themes in picker order (shared data, `data/themes.json`). */
export const presetThemes = presets as readonly AppTheme[]

/**
 * Nuxt UI's color variables for a palette, by name (`--ui-bg`, `--ui-text-muted`, …), as
 * `#rrggbb`. Colors the palette leaves out are mixed from `bg`, `text` and `primary`
 * (`crates/kchess-domain/src/theme.rs`); the GPUI app paints with the same colors.
 */
export function themeColors(palette: ThemePalette, dark: boolean): Record<string, string> {
  return rules<Record<string, string>>('themeColors', palette, dark)
}
