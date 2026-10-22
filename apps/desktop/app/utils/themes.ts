import type {
  AppTheme,
  Appearance,
  OAuthPageColors,
  OAuthPageLook,
  ThemePalette,
} from '@kchess/contracts/types'
import { presetThemes, themeColors } from '@kchess/rules/themes'

export const DEFAULT_THEME_ID = 'kchess'

/** The look KChess ships with: no palette, so the stylesheet's own colors apply. */
export const defaultTheme: AppTheme = { id: DEFAULT_THEME_ID, name: 'KChess' }

/** Default theme, then the presets, then the user's own; a custom theme replaces a preset with the same id. */
export function allThemes(custom: readonly AppTheme[] = []): AppTheme[] {
  const byId = new Map<string, AppTheme>()
  for (const theme of [defaultTheme, ...presetThemes]) byId.set(theme.id, theme)
  for (const theme of custom) byId.set(theme.id, { ...theme, custom: true })
  return [...byId.values()]
}

export function findTheme(id: string, custom: readonly AppTheme[] = []): AppTheme {
  return allThemes(custom).find((theme) => theme.id === id) ?? defaultTheme
}

/** A theme with only one variant serves both modes. */
export function paletteFor(theme: AppTheme, dark: boolean): ThemePalette | undefined {
  return dark ? (theme.dark ?? theme.light) : (theme.light ?? theme.dark)
}

/** Nuxt UI's color variables for a palette; colors left out are derived from `bg`, `text` and `primary`. */
export function themeVars(p: ThemePalette, dark: boolean): Record<string, string> {
  return themeColors(p, dark)
}

/** What the default look resembles, for the picker's preview only (the stylesheet supplies the real colors). */
const defaultPreview: Record<'light' | 'dark', ThemePalette> = {
  light: {
    bg: '#ffffff',
    elevated: '#f1f5f9',
    text: '#0f172a',
    primary: '#4b6231',
    border: '#e2e8f0',
  },
  dark: {
    bg: '#0f172a',
    elevated: '#1e293b',
    text: '#f1f5f9',
    primary: '#91af6a',
    border: '#334155',
  },
}

/** The handful of colors a theme card shows. */
export function previewColors(
  theme: AppTheme,
  dark: boolean,
): { bg: string; elevated: string; text: string; primary: string; border: string } {
  const palette = paletteFor(theme, dark) ?? defaultPreview[dark ? 'dark' : 'light']
  const vars = themeVars(palette, dark)
  return {
    bg: vars['--ui-bg'] ?? palette.bg,
    elevated: vars['--ui-bg-elevated'] ?? palette.elevated ?? palette.bg,
    text: vars['--ui-text'] ?? palette.text,
    primary: vars['--ui-primary'] ?? palette.primary,
    border: vars['--ui-border'] ?? palette.border ?? palette.bg,
  }
}

/** The app's current colors for the browser's Lichess sign-in page, in both modes. */
export function oauthPageLook(
  appearance: Appearance,
  lightTheme: AppTheme,
  darkTheme: AppTheme,
): OAuthPageLook {
  const colors = (theme: AppTheme, dark: boolean): OAuthPageColors => {
    const palette = paletteFor(theme, dark) ?? defaultPreview[dark ? 'dark' : 'light']
    const vars = themeVars(palette, dark)
    return {
      bg: vars['--ui-bg'] ?? palette.bg,
      elevated: vars['--ui-bg-elevated'] ?? palette.elevated ?? palette.bg,
      text: vars['--ui-text'] ?? palette.text,
      textMuted: vars['--ui-text-muted'] ?? palette.text,
      primary: vars['--ui-primary'] ?? palette.primary,
      border: vars['--ui-border'] ?? palette.border ?? palette.bg,
    }
  }
  return { appearance, light: colors(lightTheme, false), dark: colors(darkTheme, true) }
}

let varNames: string[] | undefined
/** Every variable a theme sets, asked of the rules once they have loaded (not at import). */
const themeVarNames = (): string[] =>
  (varNames ??= Object.keys(themeVars({ bg: '#000', text: '#fff', primary: '#888' }, true)))

/** Paint the whole app; the default theme just removes what an earlier one set. */
export function applyTheme(root: HTMLElement, theme: AppTheme, dark: boolean): void {
  const palette = paletteFor(theme, dark)
  for (const name of themeVarNames()) root.style.removeProperty(name)
  if (!palette) return
  for (const [name, value] of Object.entries(themeVars(palette, dark)))
    root.style.setProperty(name, value)
}
