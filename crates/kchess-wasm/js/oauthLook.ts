import type { OAuthPageLook } from '@kchess/contracts/types'
import { rules } from './engine.ts'

/**
 * KChess's default look (see `apps/desktop/app/utils/themes.ts`), used until the app sends its own colors.
 * The Rust rules hold the same values (`records/oauth.rs`); the golden test keeps them equal.
 */
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

/** The renderer's theme for the callback page; anything malformed falls back to the default look. */
export function oauthLook(value: unknown): OAuthPageLook {
  return rules<OAuthPageLook | null>('oauthLook', value) ?? DEFAULT_OAUTH_LOOK
}
