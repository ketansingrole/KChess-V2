import type { LichessGame } from '@kchess/contracts/types'
import { useAnalysisStore } from '../stores/analysis'
import { useKChessStore } from '../stores/kchess'
import { useReplaceAnalysis } from './useReplaceAnalysis'

/**
 * Opens a Lichess game on the analysis board from the side it was listed for, asking first when
 * it would replace unsaved analysis.
 */
export function useOpenLichessGame(): (game: LichessGame) => void {
  const analysis = useAnalysisStore()
  const store = useKChessStore()
  const replaceAnalysis = useReplaceAnalysis()
  const toast = useToast()

  async function load(game: LichessGame): Promise<void> {
    try {
      const pgn = await window.kchess.exportGame(game.id)
      if (!analysis.loadPgn(pgn)) throw new Error('That game could not be read.')
      analysis.orientation = game.color
      analysis.origin = {
        white: game.color === 'white' ? game.account : game.opponent,
        black: game.color === 'white' ? game.opponent : game.account,
        gameId: game.id,
      }
      store.selectPage('analysis')
    } catch (cause) {
      console.warn('[games] Could not open the game:', cause)
      toast.add({
        title: 'Could not open the game',
        description: cause instanceof Error ? cause.message : String(cause),
        color: 'error',
      })
    }
  }
  return (game) => replaceAnalysis(() => load(game))
}
