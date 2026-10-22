import { readJson, useViewPreference } from './useViewPreference'
import { useKChessStore } from '../stores/kchess'
import type { OpeningChoice } from './useOpeningDrill'
import {
  PRACTICE_TAB_KEY,
  RECENT_OPENINGS_KEY,
  REPERTOIRE_OPENING_KEY,
  REPERTOIRE_STUDY_KEY,
} from '../utils/storageKeys'

/** Opens Practice → Repertoire on an opening, through the Practice page's own keys. */
export function useTrainOpening(): (choice: OpeningChoice) => void {
  const store = useKChessStore()
  const practiceTab = useViewPreference(PRACTICE_TAB_KEY, 'coordinates')
  const repertoire = useViewPreference(REPERTOIRE_STUDY_KEY, '')
  const drilled = useViewPreference<OpeningChoice | null>(REPERTOIRE_OPENING_KEY, null, {
    legacy: readJson,
  })
  return (choice) => {
    drilled.value = { name: choice.name, moves: [...choice.moves] }
    repertoire.value = 'opening'
    practiceTab.value = 'openings'
    store.selectPage('practice')
  }
}

const RECENT_LIMIT = 6
/** The openings opened most recently, newest first, one line per name. */
export function useRecentOpenings() {
  const recent = useViewPreference<OpeningChoice[]>(RECENT_OPENINGS_KEY, [])
  function remember(choice: OpeningChoice): void {
    const list = Array.isArray(recent.value) ? recent.value : []
    recent.value = [
      { name: choice.name, moves: [...choice.moves] },
      ...list.filter(
        (entry) =>
          typeof entry?.name === 'string' &&
          Array.isArray(entry.moves) &&
          entry.name !== choice.name,
      ),
    ].slice(0, RECENT_LIMIT)
  }
  return { recent, remember }
}
