import { DEFAULT_OAUTH_LOOK } from '../domain/oauthLook'
import { rules } from '../domain/engine'
import type { OAuthPageLook } from '../contracts/types'

/** Self-contained callback page: no external assets, scripts, credentials, or callback values in HTML. */
export function oauthPage(authorized: boolean, look: OAuthPageLook = DEFAULT_OAUTH_LOOK): string {
  return rules<string>('oauthPage', authorized, look)
}
