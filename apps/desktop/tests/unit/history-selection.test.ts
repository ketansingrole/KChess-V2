import { expect, it, vi } from 'vitest'
import { effectScope, ref } from 'vue'
import { desktop, deferred } from './fixtures'
import { useGameHistory } from '../../app/stores/kchess/gameHistory'
import type { LichessGame, AppData, LichessRatingHistory } from '@kchess/contracts/types'
it('opening two history games retains the latest selection', async () => {
  const first = deferred<string | null>()
  desktop({ gamePgn: async (_, id) => (id === 'first' ? first.promise : '1. d4 *') })
  const scope = effectScope()
  const history = scope.run(() =>
    useGameHistory({
      data: ref<AppData | null>(null),
      selectedAccount: ref(''),
      ratingHistories: ref<LichessRatingHistory>([]),
      chartMode: ref('blitz'),
      chartRange: ref('All'),
      selectPage: vi.fn(),
    }),
  )!
  const game = (id: string) => ({ id, account: 'Alice', moves: 'e4' }) as LichessGame
  const pending = history.openReview(game('first'))
  await history.openReview(game('second'))
  expect(history.reviewGame.value?.id).toBe('second')
  first.resolve('1. e4 *')
  await pending
  scope.stop()
  expect(history.reviewGame.value?.id).toBe('second')
})

it.each(['navigate', 'dispose', 'close'] as const)(
  'does not reopen a game after %s while its PGN is pending',
  async (action) => {
    const gate = deferred<string | null>()
    desktop({ gamePgn: () => gate.promise })
    const scope = effectScope()
    const page = ref('history')
    const selectPage = vi.fn()
    const history = scope.run(() =>
      useGameHistory({
        data: ref<AppData | null>(null),
        selectedAccount: ref(''),
        ratingHistories: ref<LichessRatingHistory>([]),
        chartMode: ref('blitz'),
        chartRange: ref('All'),
        page,
        selectPage,
      }),
    )!
    history.reviewGame.value = { id: 'previous', account: 'Alice', moves: 'd4' } as LichessGame
    const opening = history.openReview({
      id: 'pending',
      account: 'Alice',
      moves: 'e4',
    } as LichessGame)
    if (action === 'navigate') page.value = 'analysis'
    else if (action === 'dispose') scope.stop()
    else history.reviewGame.value = null
    gate.resolve('1. e4 *')
    await opening
    expect(history.reviewGame.value?.id).not.toBe('pending')
    expect(selectPage).not.toHaveBeenCalled()
    scope.stop()
  },
)
