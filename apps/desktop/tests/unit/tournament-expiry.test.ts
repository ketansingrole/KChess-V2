import { afterEach, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { useTournamentStore } from '../../app/stores/tournaments'
import { useChallengeStore } from '../../app/stores/challenges'
import { useKChessStore } from '../../app/stores/kchess'
import { setInitialLibrary } from '../../app/utils/library'
import { desktop } from './fixtures'

afterEach(() => vi.useRealTimers())

it('forgets a joined tournament once it has ended', async () => {
  // `Date.now()` in a computed never re-ran: an ended arena stayed joined and active.
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-10-20T12:00:00Z'))
  desktop()
  setInitialLibrary({
    joinedTournaments: [
      {
        system: 'arena',
        id: 'Arena001',
        account: 'alice',
        name: 'Hourly Blitz',
        until: Date.parse('2026-10-20T12:30:00Z'),
      },
    ],
  })
  const tournaments = useTournamentStore()
  expect(tournaments.active).toBe(true)
  vi.advanceTimersByTime(31 * 60_000)
  await nextTick()
  expect(tournaments.active).toBe(false)
})

// Memberships ignored the account: after joining as Alice and selecting Bob, the arena showed
// as joined for Bob, and the one event stream moved to Bob so Alice's pairings stopped arriving.
it('keeps a membership and its event stream with the account that joined', async () => {
  const stayConnected = vi.fn(async () => {})
  desktop({
    stayConnected,
    onChallenges: () => vi.fn(),
    onLobbyState: () => vi.fn(),
    onOngoingChanged: () => vi.fn(),
    challenges: async () => [],
    ongoingGames: async () => [],
  })
  setInitialLibrary({
    joinedTournaments: [
      { system: 'arena', id: 'Arena001', account: 'Alice', name: 'Blitz', until: Date.now() + 3e6 },
    ],
  })
  const app = useKChessStore()
  await app.init()
  app.setOnlineAccount('Alice')
  const tournaments = useTournamentStore()
  const challenges = useChallengeStore()
  challenges.start()
  expect(tournaments.isJoined('arena', 'Arena001')).toBe(true)
  expect(stayConnected).toHaveBeenLastCalledWith('Alice')

  app.setOnlineAccount('Bob')
  await nextTick()
  expect(app.activeOnlineAccount).toBe('Bob')
  expect(tournaments.isJoined('arena', 'Arena001')).toBe(false)
  expect(tournaments.isJoined('arena', 'Arena001', 'alice')).toBe(true)
  expect(tournaments.participant).toBe('Alice')
  expect(stayConnected).not.toHaveBeenCalledWith('Bob')
  challenges.stop()
})

it('opens the challenge stream again after the core restarts', async () => {
  const stayConnected = vi.fn(async () => {})
  const challengesRead = vi.fn(async () => [])
  let reconnected = () => {}
  desktop({
    stayConnected,
    onChallenges: () => vi.fn(),
    onLobbyState: () => vi.fn(),
    onOngoingChanged: () => vi.fn(),
    onCoreReconnected: (callback: () => void) => {
      reconnected = callback
      return vi.fn()
    },
    challenges: challengesRead,
    ongoingGames: async () => [],
  })
  const app = useKChessStore()
  await app.init()
  app.setOnlineAccount('Alice')
  app.settings!.receiveChallenges = true
  const challenges = useChallengeStore()
  challenges.start()
  expect(stayConnected).toHaveBeenLastCalledWith('Alice')
  stayConnected.mockClear()
  challengesRead.mockClear()
  // The idle window's account is unchanged; only the core restarted.
  reconnected()
  expect(stayConnected).toHaveBeenCalledWith('Alice')
  expect(challengesRead).toHaveBeenCalledOnce()
  challenges.stop()
})
