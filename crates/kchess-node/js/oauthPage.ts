import { DEFAULT_OAUTH_LOOK } from '@kchess/rules/oauthLook'
import { rules } from '@kchess/rules/engine'
import type { OAuthPageLook } from '@kchess/contracts/types'

/** Self-contained callback page: no external assets, scripts, credentials, or callback values in HTML. */
export function oauthPage(authorized: boolean, look: OAuthPageLook = DEFAULT_OAUTH_LOOK): string {
  return rules<string>('oauthPage', authorized, look)
}
