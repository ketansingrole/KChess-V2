import type { MistakeGame } from '@kchess/rules/library'
import { setupPgn } from '@kchess/rules/chess'
import { useAnalysisStore } from '../stores/analysis'
import { useKChessStore } from '../stores/kchess'
import { useReviewPractice } from '../stores/reviewPractice'
import { useReplaceAnalysis } from './useReplaceAnalysis'

/**
 * Practises a reviewed game's positions on the analysis board, with the game around them. A game
 * already on the board is kept as it is (variations and all); another replaces it, asking first
 * when that would lose unsaved analysis.
 */
export function usePracticeGame(): (game: MistakeGame) => void {
  const analysis = useAnalysisStore()
  const practice = useReviewPractice()
  const store = useKChessStore()
  const replaceAnalysis = useReplaceAnalysis()
  function begin(game: MistakeGame): void {
    store.selectPage('analysis')
    practice.start(game.key)
    analysis.tool = 'moves'
  }
  return (game) => {
    if (analysis.mainlineKey === game.key) return begin(game)
    replaceAnalysis(() => {
      const headers: Record<string, string> = {}
      if (game.white) headers.White = game.white
      if (game.black) headers.Black = game.black
      if (!analysis.loadPgn(setupPgn({ variant: 'standard', fen: game.fen }, game.moves, headers)))
        return
      analysis.origin = {
        gameId: game.gameId,
        account: game.account,
        white: game.white,
        black: game.black,
        player: game.player,
      }
      begin(game)
    })
  }
}
