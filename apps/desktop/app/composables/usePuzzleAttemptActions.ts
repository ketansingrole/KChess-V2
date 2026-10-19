import type { Puzzle } from '@kchess/contracts/types'
import { playerColor } from '@kchess/rules/puzzle'
import { useAnalysisStore } from '../stores/analysis'
import { usePuzzleStore } from '../stores/puzzles'
import { useReplaceAnalysis } from './useReplaceAnalysis'

/** What an earlier puzzle offers, wherever it is listed: try it again, or study it on the board. */
export function usePuzzleAttemptActions(): {
  retry: (puzzle: Puzzle) => void
  analyse: (puzzle: Puzzle) => void
  menu: (puzzle: Puzzle) => { label: string; icon: string; onSelect: () => void }[]
} {
  const puzzles = usePuzzleStore()
  const analysis = useAnalysisStore()
  const replaceAnalysis = useReplaceAnalysis()
  const retry = (puzzle: Puzzle): void => puzzles.retry(puzzle)
  const analyse = (puzzle: Puzzle): void =>
    replaceAnalysis(() => {
      analysis.load(puzzle.fen)
      analysis.orientation = playerColor(puzzle)
      void navigateTo('/analysis')
    })
  return {
    retry,
    analyse,
    menu: (puzzle) => [
      { label: 'Try again', icon: 'i-lucide-rotate-ccw', onSelect: () => retry(puzzle) },
      { label: 'Analysis board', icon: 'i-lucide-microscope', onSelect: () => analyse(puzzle) },
    ],
  }
}
