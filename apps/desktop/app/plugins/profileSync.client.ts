import type { ProfileArea } from '@kchess/contracts/types'
import { RequestScope } from '@kchess/rules/requestScope'
import { setLibraryRefresher } from '../utils/library'
import { useStudyStore } from '../stores/studies'
import { useGameArchiveStore } from '../stores/gameArchive'
import { useMistakeStore } from '../stores/mistakes'
import { useTournamentStore } from '../stores/tournaments'
import { useKChessStore } from '../stores/kchess'

/** The library's documents: one read of the library refreshes them all. */
const LIBRARY: ProfileArea[] = [
  'studies',
  'playedGames',
  'drills',
  'sessions',
  'tournaments',
  'repertoire',
]
const ACCOUNTS: ProfileArea[] = ['accounts', 'lichessGames', 'usage', 'desktopSettings']

/**
 * Another KChess window on this profile changed it (`docs/shared-core.md`): show the profile as
 * it now is. This window's own writes are never reported here.
 */
export default defineNuxtPlugin(() => {
  if (!window.kchess) return
  // One read at a time is shown: a reply older than the newest read is dropped.
  const reads = new RequestScope()
  function refreshLibrary(): void {
    const read = reads.next()
    // Played games written here after this read starts make it stale for them.
    const archive = useGameArchiveStore()
    const since = archive.revision()
    void window.kchess
      .library()
      .then((snapshot) => {
        if (!read.current()) return
        useStudyStore().replaceFromLibrary(snapshot)
        archive.replaceFromLibrary(snapshot, since)
        useMistakeStore().replaceFromLibrary(snapshot)
        useTournamentStore().replaceFromLibrary(snapshot)
      })
      .catch((cause: unknown) => console.warn('[library] refreshing the library failed:', cause))
  }
  setLibraryRefresher(refreshLibrary)
  window.kchess.onProfileChanged(({ areas }) => {
    if (areas.some((area) => LIBRARY.includes(area))) refreshLibrary()
    if (areas.some((area) => ACCOUNTS.includes(area))) void useKChessStore().reloadData()
  })
  // Changes made while this window was disconnected were never reported: read everything again.
  window.kchess.onCoreReconnected(() => {
    refreshLibrary()
    void useKChessStore().reloadData()
  })
})
