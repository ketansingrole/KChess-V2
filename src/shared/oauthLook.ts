import { APPEARANCES, type OAuthPageColors, type OAuthPageLook } from './types'

/** KChess's default look (see `app/utils/themes.ts`), used until the app sends its own colors. */
export const DEFAULT_OAUTH_LOOK: OAuthPageLook = {
  appearance: 'system',
  light: {
    bg: '#ffffff',
    elevated: '#f1f5f9',
    text: '#0f172a',
    textMuted: '#64748b',
    primary: '#4b6231',
    border: '#e2e8f0',
  },
  dark: {
    bg: '#0f172a',
    elevated: '#1e293b',
    text: '#f1f5f9',
    textMuted: '#94a3b8',
    primary: '#91af6a',
    border: '#334155',
  },
}

const COLOR_KEYS = ['bg', 'elevated', 'text', 'textMuted', 'primary', 'border'] as const
/** Hex colors and `color-mix(in srgb, …)` of them; no `;`, braces or `<` can leave the declaration. */
const CSS_COLOR = /^(?:#[0-9a-f]{3,8}|color-mix\(in srgb,[#0-9a-z(),.% -]+\))$/i

function colors(value: unknown): OAuthPageColors | undefined {
  if (!value || typeof value !== 'object') return undefined
  const result: Partial<OAuthPageColors> = {}
  for (const key of COLOR_KEYS) {
    const color = (value as Record<string, unknown>)[key]
    if (typeof color !== 'string' || color.length > 240 || !CSS_COLOR.test(color)) return undefined
    result[key] = color
  }
  return result as OAuthPageColors
}

/** The renderer's theme for the callback page; anything malformed falls back to the default look. */
export function oauthLook(value: unknown): OAuthPageLook {
  if (!value || typeof value !== 'object') return DEFAULT_OAUTH_LOOK
  const { appearance, light, dark } = value as Record<string, unknown>
  const lightColors = colors(light)
  const darkColors = colors(dark)
  if (!lightColors || !darkColors || !APPEARANCES.includes(appearance as never))
    return DEFAULT_OAUTH_LOOK
  return {
    appearance: appearance as OAuthPageLook['appearance'],
    light: lightColors,
    dark: darkColors,
  }
}
