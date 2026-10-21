import { describe, expect, it, vi } from 'vitest'
import { effectScope } from 'vue'
import { INITIAL_FEN } from '@kchess/rules/position'
import { useEngineMove } from '../../app/composables/useEngineMove'
import { bestMoveOwned } from '../../app/utils/engineControl'
import { deferred, desktop } from './fixtures'

// Training's cancel only ignored the reply: the core search kept its engine lease, and a search
// outstanding when Endgame Drills closed still returned a move.
describe('training engine moves', () => {
  it('stops its own search on cancel and on disposal, and drops the move', async () => {
    const replies: ReturnType<typeof deferred<string>>[] = []
    const stopEngine = vi.fn(async () => {})
    desktop({
      bestMove: () => {
        const reply = deferred<string>()
        replies.push(reply)
        return reply.promise
      },
      stopEngine,
    })
    const scope = effectScope()
    const engine = scope.run(() => useEngineMove('test'))!
    const first = engine.ask(INITIAL_FEN, [], 350)
    expect(engine.thinking.value).toBe(true)
    engine.cancel()
    expect(stopEngine).toHaveBeenCalledTimes(1)
    replies[0]!.resolve('e2e4')
    expect(await first).toBeUndefined()

    const second = engine.ask(INITIAL_FEN, [], 350)
    scope.stop()
    expect(stopEngine).toHaveBeenCalledTimes(2)
    replies[1]!.resolve('d2d4')
    expect(await second).toBeUndefined()
  })

  it('leaves a search another caller started since', async () => {
    const stopEngine = vi.fn(async () => {})
    desktop({ bestMove: () => new Promise<string>(() => {}), stopEngine })
    const scope = effectScope()
    const engine = scope.run(() => useEngineMove('old'))!
    void engine.ask(INITIAL_FEN, [], 350)
    // The replacement view (or the computer game) supersedes the old view's search.
    void bestMoveOwned([], 'max', { fen: INITIAL_FEN, movetime: 350 })
    scope.stop()
    expect(stopEngine).not.toHaveBeenCalled()
  })
})
