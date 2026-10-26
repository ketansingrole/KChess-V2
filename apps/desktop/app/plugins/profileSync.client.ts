import type { ProfileArea } from '@kchess/contracts/types'
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
const ACCOUNTS: ProfileArea[] = ['accounts', 'lichessGames', 'usage']

/**
 * Another KChess window on this profile changed it (`docs/shared-core.md`): show the profile as
 * it now is. This window's own writes are never reported here.
 */
export default defineNuxtPlugin(() => {
  window.kchess?.onProfileChanged(({ areas }) => {
    if (areas.some((area) => LIBRARY.includes(area)))
      void window.kchess
        .library()
        .then((snapshot) => {
          useStudyStore().replaceFromLibrary(snapshot)
          useGameArchiveStore().replaceFromLibrary(snapshot)
          useMistakeStore().replaceFromLibrary(snapshot)
          useTournamentStore().replaceFromLibrary(snapshot)
        })
        .catch((cause: unknown) => console.warn('[library] refreshing the library failed:', cause))
    if (areas.some((area) => ACCOUNTS.includes(area))) void useKChessStore().reloadData()
  })
})
