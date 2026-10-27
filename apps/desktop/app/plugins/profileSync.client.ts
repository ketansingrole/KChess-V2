import type { ProfileArea } from '@kchess/contracts/types'
import { RequestScope } from '@kchess/rules/requestScope'
import { setLibraryRefresher } from '../utils/library'
import { useStudyStore } from '../stores/studies'
import { useGameArchiveStore } from '../stores/gameArchive'
import { useMistakeStore } from '../stores/mistakes'
import { useTournamentStore } from '../stores/tournaments'
import { useKChessStore } from '../stores/kchess'
import { loadViewPreferences } from '../composables/useViewPreference'

type LibraryStore = 'studies' | 'archive' | 'mistakes' | 'tournaments'
const ALL_STORES: LibraryStore[] = ['studies', 'archive', 'mistakes', 'tournaments']
/**
 * The store each library area is shown by. Sessions are only read when a page opens
 * (`initialLibrary`), so another window's session save, made after every move, needs no read.
 */
const LIBRARY: Partial<Record<ProfileArea, LibraryStore>> = {
  studies: 'studies',
  repertoire: 'studies',
  playedGames: 'archive',
  drills: 'mistakes',
  tournaments: 'tournaments',
}
const ACCOUNTS: ProfileArea[] = ['accounts', 'lichessGames', 'usage', 'desktopSettings']

/**
 * Another KChess window on this profile changed it (`docs/shared-core.md`): show the profile as
 * it now is. This window's own writes are never reported here.
 */
export default defineNuxtPlugin(() => {
  if (!window.kchess) return
  // One read at a time is shown: a reply older than the newest read is dropped.
  const reads = new RequestScope()
  // Stores still waiting for a read: a newer read replaces an older one's reply, so it shows
  // every store either was for.
  const pending = new Set<LibraryStore>()
  function refreshLibrary(stores: LibraryStore[] = ALL_STORES): void {
    for (const store of stores) pending.add(store)
    const read = reads.next()
    // A store's own writes made after this read starts make it stale for that store.
    const studies = useStudyStore()
    const archive = useGameArchiveStore()
    const mistakes = useMistakeStore()
    const tournaments = useTournamentStore()
    const owners = { studies, archive, mistakes, tournaments }
    const since = {
      studies: studies.revision(),
      archive: archive.revision(),
      mistakes: mistakes.revision(),
      tournaments: tournaments.revision(),
    }
    void window.kchess
      .library()
      .then((snapshot) => {
        if (!read.current()) return
        // A store may ask for another read while it takes this one (`refreshLibraryLater`): that
        // read's stores stay pending, and this older snapshot is not shown to the rest.
        const stores = [...pending]
        pending.clear()
        for (const [index, store] of stores.entries()) {
          if (!read.current()) {
            for (const rest of stores.slice(index)) pending.add(rest)
            return
          }
          owners[store].replaceFromLibrary(snapshot, since[store])
        }
      })
      .catch((cause: unknown) => console.warn('[library] refreshing the library failed:', cause))
  }
  setLibraryRefresher(() => refreshLibrary())
  window.kchess.onProfileChanged(({ areas }) => {
    const stores = areas.flatMap((area) => LIBRARY[area] ?? [])
    if (stores.length) refreshLibrary(stores)
    if (areas.some((area) => ACCOUNTS.includes(area))) void useKChessStore().reloadData()
    if (areas.includes('viewPreferences')) void loadViewPreferences()
  })
  // Changes made while this window was disconnected were never reported: read everything again.
  window.kchess.onCoreReconnected(() => {
    refreshLibrary()
    void useKChessStore().reloadData()
    void loadViewPreferences()
  })
})
