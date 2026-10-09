/** Choosing which connected Lichess account plays online; shared by the UI and Electron main. */

interface AccountLike {
  username: string
  connected: boolean
}

/**
 * The connected account matching `preferred` (case-insensitive), or the first
 * connected one when `preferred` is empty or no longer connected.
 */
export function pickConnectedAccount<T extends AccountLike>(
  accounts: readonly T[],
  preferred?: string,
): T | undefined {
  const connected = accounts.filter((account) => account.connected)
  const wanted = preferred?.toLowerCase()
  return (
    (wanted ? connected.find((account) => account.username.toLowerCase() === wanted) : undefined) ??
    connected[0]
  )
}
