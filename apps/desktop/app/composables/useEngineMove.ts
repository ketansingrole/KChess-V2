import { ref } from 'vue'

/**
 * Stockfish at full strength for practice positions: its replies and hints from a start FEN and
 * the moves played since. A move asked for before `cancel()` (a restart, a takeback, another
 * position) is dropped when it arrives.
 */
export function useEngineMove(label: string) {
  const thinking = ref(false)
  const error = ref('')
  let epoch = 0

  /** The best move after `moves` from `fen`, or undefined when cancelled or failed. */
  async function ask(
    fen: string,
    moves: readonly string[],
    movetime: number,
    { busy = true } = {},
  ): Promise<string | undefined> {
    const mine = epoch
    if (busy) thinking.value = true
    try {
      const best = await window.kchess.bestMove([...moves], 'max', { fen, movetime })
      return mine === epoch ? best : undefined
    } catch (cause) {
      if (mine === epoch) {
        console.warn(`[${label}] Engine move failed:`, cause)
        error.value = cause instanceof Error ? cause.message : String(cause)
      }
      return undefined
    } finally {
      if (busy && mine === epoch) thinking.value = false
    }
  }

  function cancel(): void {
    epoch++
    thinking.value = false
    error.value = ''
  }

  return { thinking, error, ask, cancel }
}
