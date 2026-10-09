/** Choosing which connected Lichess account plays online; shared by the UI and Electron main. */
import { rules } from './engine.ts'

interface AccountLike {
  username: string
  connected: boolean
}

/**
 * The connected account matching `preferred` (case-insensitive), or the first
 * connected one when `preferred` is empty or no longer connected. The rules choose the
 * position in `accounts` (`records/accounts.rs`), so the caller gets its own object back.
 */
export function pickConnectedAccount<T extends AccountLike>(
  accounts: readonly T[],
  preferred?: string,
): T | undefined {
  const index = rules<number | null>(
    'connectedAccountIndex',
    accounts.map(({ username, connected }) => ({ username, connected })),
    preferred ?? null,
  )
  return index === null ? undefined : accounts[index]
}
