import { computed, onScopeDispose, shallowReactive, watch } from 'vue'
import type { AnalysisRequest, BroadcastGame, EngineLine } from '../../src/shared/types'
import { analysisContext } from '../../src/shared/analysisContext'
import { usePositionEngine } from './usePositionEngine'

interface Preview {
  context: string
  /** A sampled estimate, not a completed deep search. */
  line?: EngineLine
}

/** Share one engine between the open board and short, sequential grid previews. */
export function useBroadcastEvaluations(
  detail: () => AnalysisRequest | null,
  games: () => BroadcastGame[],
  identity: () => string,
) {
  const previews = shallowReactive(new Map<string, Preview>())
  const context = (game: BroadcastGame) => analysisContext(game.startFen, game.moves)
  const selected = computed(() =>
    games().find((game) => game.ongoing && previews.get(game.id)?.context !== context(game)),
  )
  const request = computed<AnalysisRequest | null>(() => {
    const open = detail()
    if (open) return open
    const game = selected.value
    return game ? { fen: game.fen, rootFen: game.startFen, moves: game.moves, lines: 1 } : null
  })
  const engine = usePositionEngine(() => request.value)
  let timer: ReturnType<typeof setTimeout> | undefined

  function finish(): void {
    if (detail()) return
    const game = selected.value
    if (!game) return
    const value = engine.update.value
    previews.set(game.id, {
      context: context(game),
      line: value?.context === context(game) ? value.lines[0] : undefined,
    })
  }
  watch(
    request,
    (value) => {
      clearTimeout(timer)
      if (value && !detail()) timer = setTimeout(finish, 1500)
    },
    { immediate: true, flush: 'sync' },
  )
  watch(engine.update, (value) => {
    const game = selected.value
    if (!detail() && game && value?.context === context(game) && (value.depth >= 12 || value.done))
      finish()
  })
  watch(engine.error, (error) => {
    if (error) finish()
  })
  watch(identity, () => previews.clear())
  watch(games, (entries) => {
    const ids = new Set(entries.map((game) => game.id))
    for (const id of previews.keys()) if (!ids.has(id)) previews.delete(id)
  })
  onScopeDispose(() => clearTimeout(timer))

  function evaluation(game: BroadcastGame): EngineLine | undefined {
    const key = context(game)
    if (!detail() && selected.value?.id === game.id && engine.update.value?.context === key)
      return engine.update.value.lines[0]
    const preview = previews.get(game.id)
    return preview?.context === key ? preview.line : undefined
  }
  function unavailable(game: BroadcastGame): boolean {
    const preview = previews.get(game.id)
    return preview?.context === context(game) && !preview.line
  }
  return { ...engine, evaluation, unavailable }
}
