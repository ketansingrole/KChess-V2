import { afterEach, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { useTournamentStore } from '../../app/stores/tournaments'
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
