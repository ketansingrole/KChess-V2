import type { Puzzle, PuzzleWrongMove } from '@kchess/contracts/types'
import { usePuzzleAnalysis } from '../stores/puzzleAnalysis'
import { usePuzzleStore } from '../stores/puzzles'
import { useReplaceAnalysis } from './useReplaceAnalysis'

/**
 * What an earlier puzzle offers, wherever it is listed: try it again, or study it on the board
 * with its solution and the moves tried in that attempt.
 */
export function usePuzzleAttemptActions(): {
  retry: (puzzle: Puzzle) => void
  analyse: (puzzle: Puzzle, wrongMoves?: readonly PuzzleWrongMove[]) => void
  menu: (
    puzzle: Puzzle,
    wrongMoves?: readonly PuzzleWrongMove[],
  ) => { label: string; icon: string; onSelect: () => void }[]
} {
  const puzzles = usePuzzleStore()
  const studied = usePuzzleAnalysis()
  const replaceAnalysis = useReplaceAnalysis()
  const retry = (puzzle: Puzzle): void => puzzles.retry(puzzle)
  const analyse = (puzzle: Puzzle, wrongMoves?: readonly PuzzleWrongMove[]): void =>
    replaceAnalysis(() => {
      if (studied.open(puzzle, wrongMoves)) void navigateTo('/analysis')
    })
  return {
    retry,
    analyse,
    menu: (puzzle, wrongMoves) => [
      { label: 'Try again', icon: 'i-lucide-rotate-ccw', onSelect: () => retry(puzzle) },
      {
        label: 'Analysis board',
        icon: 'i-lucide-microscope',
        onSelect: () => analyse(puzzle, wrongMoves),
      },
    ],
  }
}
