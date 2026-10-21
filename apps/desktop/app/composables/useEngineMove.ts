import { onScopeDispose, ref } from 'vue'
import { RequestScope } from '@kchess/rules/requestScope'
import { bestMoveOwned, stopOwnedSearch } from '../utils/engineControl'

/**
 * Stockfish at full strength for practice positions: its replies and hints from a start FEN and
 * the moves played since. `cancel()` (a restart, a takeback, another position) and leaving the
 * view stop the search this helper started and drop its move.
 */
export function useEngineMove(label: string) {
  const thinking = ref(false)
  const error = ref('')
  const requests = new RequestScope()
  let ticket = 0

  /** The best move after `moves` from `fen`, or undefined when cancelled or failed. */
  async function ask(
    fen: string,
    moves: readonly string[],
    movetime: number,
    { busy = true } = {},
  ): Promise<string | undefined> {
    const mine = requests.capture()
    if (busy) thinking.value = true
    try {
      const search = bestMoveOwned([...moves], 'max', { fen, movetime })
      ticket = search.ticket
      const best = await search.result
      return mine.current() ? best : undefined
    } catch (cause) {
      if (mine.current()) {
        console.warn(`[${label}] Engine move failed:`, cause)
        error.value = cause instanceof Error ? cause.message : String(cause)
      }
      return undefined
    } finally {
      if (busy && mine.current()) thinking.value = false
    }
  }

  function cancel(): void {
    requests.invalidate()
    stopOwnedSearch(label, ticket)
    thinking.value = false
    error.value = ''
  }
  onScopeDispose(cancel)

  return { thinking, error, ask, cancel }
}
