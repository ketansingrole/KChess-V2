import { APP_ICON_SVG } from './appIconSvg'
import { APPEARANCES, type OAuthPageColors, type OAuthPageLook } from '../shared/types'

/** KChess's default look (see `app/utils/themes.ts`), used until the app sends its own colors. */
export const DEFAULT_OAUTH_LOOK: OAuthPageLook = {
  appearance: 'system',
  light: {
    bg: '#ffffff',
    elevated: '#f1f5f9',
    text: '#0f172a',
    textMuted: '#64748b',
    primary: '#718d4d',
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

const vars = (c: OAuthPageColors): string =>
  `--bg:${c.bg};--card:${c.elevated};--text:${c.text};--muted:${c.textMuted};--primary:${c.primary};--border:${c.border};`

/** Self-contained callback page: no external assets, scripts, credentials, or callback values in HTML. */
export function oauthPage(authorized: boolean, look: OAuthPageLook = DEFAULT_OAUTH_LOOK): string {
  const title = authorized ? 'Signed in to Lichess' : 'Connection cancelled'
  const message = authorized
    ? 'KChess is finishing the connection and has been brought back to the front.'
    : 'Your Lichess account was not connected. Return to KChess to try again when you’re ready.'
  const hint = authorized
    ? 'You can close this browser tab and continue in KChess.'
    : 'You can close this browser tab.'
  const theme =
    look.appearance === 'system'
      ? `:root{color-scheme:light;${vars(look.light)}}@media(prefers-color-scheme:dark){:root{color-scheme:dark;${vars(look.dark)}}}`
      : `:root{color-scheme:${look.appearance};${vars(look[look.appearance])}}`
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="referrer" content="no-referrer"><title>${title} · KChess</title>
<style>
${theme}
*{box-sizing:border-box}html{font-family:Inter,ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif;-webkit-font-smoothing:antialiased}body{margin:0;min-height:100vh;display:grid;place-items:center;padding:24px;background:radial-gradient(ellipse at top,color-mix(in srgb,var(--primary) 14%,transparent) 0,transparent 60%),var(--bg);color:var(--text)}main{width:100%;max-width:440px;padding:36px;border:1px solid var(--border);border-radius:16px;background:var(--card)}.brand{display:flex;align-items:center;gap:8px;font-size:15px;font-weight:600;color:var(--muted)}.brand svg{width:24px;height:24px;flex:none}.mark{display:grid;place-items:center;width:56px;height:56px;margin:28px 0 20px;border-radius:50%;background:color-mix(in srgb,var(--primary) 16%,transparent);color:var(--primary);font-size:26px;font-weight:700}h1{font-size:24px;line-height:1.25;margin:0 0 12px;letter-spacing:-.02em}p{font-size:15px;line-height:1.6;color:color-mix(in srgb,var(--text) 80%,var(--muted));margin:0 0 20px}.hint{border-top:1px solid var(--border);padding-top:20px;margin:0;font-size:13px;color:var(--muted)}@media(max-width:480px){main{padding:24px}h1{font-size:22px}}
</style></head><body><main><div class="brand">${APP_ICON_SVG.replace('<svg ', '<svg aria-hidden="true" ')}KChess</div><div class="mark" aria-hidden="true">${authorized ? '✓' : '↩'}</div><h1>${title}</h1><p>${message}</p><p class="hint">${hint}</p></main></body></html>`
}
