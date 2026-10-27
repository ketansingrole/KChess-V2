import { getCurrentScope, onScopeDispose } from 'vue'
import { RequestScope } from '@kchess/rules/requestScope'
import { useAnalysisStore } from '../stores/analysis'
import { useKChessStore } from '../stores/kchess'

/** The latest replacement of the board: a newer one, from any caller, supersedes a download. */
const handoffs = new RequestScope()

export type ReplaceAnalysis = ((load: () => void | Promise<void>) => void) & {
  /**
   * Replace the board with what `fetch` downloads. A reply superseded by another replacement is
   * dropped, and one arriving after the board was edited asks again before replacing that edit.
   * `failed` tells the person; the failure is already logged.
   */
  after<T>(
    fetch: (signal: AbortSignal) => Promise<T>,
    apply: (value: T) => void,
    failed: (cause: unknown) => void,
  ): void
}

/**
 * Puts something else on the analysis board, asking first while the board holds moves that no
 * study keeps: a game, a position or a study must never silently throw that analysis away.
 */
export function useReplaceAnalysis(): ReplaceAnalysis {
  const analysis = useAnalysisStore()
  const store = useKChessStore()
  let mine: ReturnType<RequestScope['capture']> | undefined
  if (getCurrentScope())
    onScopeDispose(() => {
      // Leaving the page that asked drops its download, not another caller's.
      if (mine?.current()) handoffs.invalidate()
    })
  const guard = (run: () => void): void => {
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
  const board = (): string => `${analysis.study?.id ?? ''}\n${analysis.pgn()}`
  const replace = (load: () => void | Promise<void>): void =>
    guard(() => {
      handoffs.invalidate()
      void load()
    })
  return Object.assign(replace, {
    after<T>(
      fetch: (signal: AbortSignal) => Promise<T>,
      apply: (value: T) => void,
      failed: (cause: unknown) => void,
    ): void {
      guard(() => {
        const request = (mine = handoffs.next())
        const before = board()
        const fail = (cause: unknown): void => {
          console.warn('[analysis] Could not replace the board:', cause)
          failed(cause)
        }
        const adopt = (value: T): void => {
          if (!request.current()) return
          try {
            apply(value)
          } catch (cause) {
            console.warn('[analysis] Could not replace the board:', cause)
            failed(cause)
          }
        }
        fetch(request.signal).then(
          (value) => {
            if (!request.current()) return
            if (board() === before) adopt(value)
            else guard(() => adopt(value))
          },
          (cause: unknown) => {
            if (request.current()) fail(cause)
          },
        )
      })
    },
  })
}
