import { useAnalysisStore } from '../stores/analysis'
import { useKChessStore } from '../stores/kchess'

/**
 * Puts something else on the analysis board, asking first while the board holds moves that no
 * study keeps: a game, a position or a study must never silently throw that analysis away.
 */
export function useReplaceAnalysis(): (load: () => void | Promise<void>) => void {
  const analysis = useAnalysisStore()
  const store = useKChessStore()
  return (load) => {
    const run = (): void => void load()
    if (!analysis.study && analysis.root.children.length > 0)
      store.ask({
        title: 'Replace the unsaved analysis?',
        description: 'The analysis board has moves that are not saved as a study.',
        label: 'Replace',
        color: 'error',
        run,
      })
    else run()
  }
}
