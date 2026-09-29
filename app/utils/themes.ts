import type { AppTheme, ThemePalette } from '../../src/shared/types'
import { presetThemes } from './themePresets'

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

const mix = (a: string, share: number, b: string): string =>
  `color-mix(in srgb, ${a} ${share}%, ${b})`

/** Nuxt UI's color variables for a palette; colors left out are derived from `bg`, `text` and `primary`. */
export function themeVars(p: ThemePalette, dark: boolean): Record<string, string> {
  const muted = p.muted ?? mix(p.bg, 95, p.text)
  const elevated = p.elevated ?? mix(p.bg, 91, p.text)
  const accented = p.accented ?? mix(p.bg, 84, p.text)
  const border = p.border ?? mix(p.bg, 82, p.text)
  const textMuted = p.textMuted ?? mix(p.text, 62, p.bg)
  const success = p.success ?? (dark ? '#62b57a' : '#3f8f58')
  const error = p.error ?? (dark ? '#e07979' : '#c74e4e')
  return {
    '--ui-bg': p.bg,
    '--ui-bg-muted': muted,
    '--ui-bg-elevated': elevated,
    '--ui-bg-accented': accented,
    '--ui-bg-inverted': p.text,
    '--ui-border': border,
    '--ui-border-muted': mix(border, 55, p.bg),
    '--ui-border-accented': mix(border, 65, p.text),
    '--ui-border-inverted': p.text,
    '--ui-text': p.text,
    '--ui-text-highlighted': p.text,
    '--ui-text-toned': mix(p.text, 80, textMuted),
    '--ui-text-muted': textMuted,
    '--ui-text-dimmed': mix(textMuted, 60, p.bg),
    '--ui-text-inverted': p.bg,
    '--ui-primary': p.primary,
    '--ui-success': success,
    '--ui-warning': p.warning ?? (dark ? '#e0b25c' : '#b8862b'),
    '--ui-error': error,
    '--ui-info': p.info ?? (dark ? '#6cb6ff' : '#2f7fd0'),
    '--ui-win': success,
    '--ui-loss': error,
  }
}

/** What the default look resembles, for the picker's preview only (the stylesheet supplies the real colors). */
const defaultPreview: Record<'light' | 'dark', ThemePalette> = {
  light: {
    bg: '#ffffff',
    elevated: '#f1f5f9',
    text: '#0f172a',
    primary: '#718d4d',
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
    bg: vars['--ui-bg']!,
    elevated: vars['--ui-bg-elevated']!,
    text: vars['--ui-text']!,
    primary: vars['--ui-primary']!,
    border: vars['--ui-border']!,
  }
}

const VAR_NAMES = Object.keys(themeVars({ bg: '#000', text: '#fff', primary: '#888' }, true))

/** Paint the whole app; the default theme just removes what an earlier one set. */
export function applyTheme(root: HTMLElement, theme: AppTheme, dark: boolean): void {
  const palette = paletteFor(theme, dark)
  for (const name of VAR_NAMES) root.style.removeProperty(name)
  if (!palette) return
  for (const [name, value] of Object.entries(themeVars(palette, dark)))
    root.style.setProperty(name, value)
}
