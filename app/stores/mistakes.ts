import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { Puzzle, StoredReview } from '../../src/shared/types'
import { analyseReview, replay } from '../../src/shared/review'
import { readSession, persistSession } from '../utils/sessionPersistence'
interface Exercise {
  id: string
  fen: string
  solution: string[]
  judgment: string
  dueAt: number
  streak: number
  attempts: number
}
export const useMistakeStore = defineStore('mistakes', () => {
  const items = ref<Exercise[]>(
    readSession('kchess:mistakes:v1', (raw) => {
      if (
        !raw ||
        typeof raw !== 'object' ||
        !('version' in raw) ||
        raw.version !== 1 ||
        !('items' in raw) ||
        !Array.isArray(raw.items)
      )
        return undefined
      return raw.items
        .slice(0, 500)
        .filter((item): item is Exercise =>
          Boolean(
            item &&
            typeof item.id === 'string' &&
            typeof item.fen === 'string' &&
            Array.isArray(item.solution) &&
            item.solution.every((move: unknown) => typeof move === 'string') &&
            replay(item.fen, item.solution).length === item.solution.length + 1 &&
            Number.isFinite(item.dueAt) &&
            Number.isInteger(item.streak) &&
            Number.isInteger(item.attempts),
          ),
        )
    }) ?? [],
  )
  const error = persistSession('kchess:mistakes:v1', () => ({ version: 1, items: items.value }))
  function add(review: StoredReview, color?: 'white' | 'black'): number {
    if (!review.complete) return 0
    const positions = replay(review.fen, review.moves)
    const analysis = analyseReview(review)
    let added = 0
    for (const [index, move] of analysis.moves.entries()) {
      if (!move.judgment || (color && move.color !== color)) continue
      const evaluation = review.evals[index]
      const best = evaluation?.best
      const fen = positions[index]?.fen
      if (
        !best ||
        !fen ||
        best === review.moves[index] ||
        (review.source === 'local' && (evaluation.depth ?? 0) < 10)
      )
        continue
      let solution = evaluation.pv?.[0] === best ? evaluation.pv.slice(0, 7) : [best]
      solution = solution.slice(0, replay(fen, solution).length - 1)
      if (solution.length % 2 === 0) solution.pop()
      if (!solution.length) continue
      const id = `${review.key}:${index}`
      if (items.value.some((item) => item.id === id)) continue
      items.value.push({
        id,
        fen,
        solution,
        judgment: move.judgment,
        dueAt: Date.now(),
        streak: 0,
        attempts: 0,
      })
      added++
    }
    items.value = items.value.slice(-500)
    return added
  }
  function puzzle(item: Exercise): Puzzle {
    return { id: item.id, fen: item.fen, solution: [...item.solution], rating: 0, themes: [] }
  }
  function answered(id: string, clean: boolean): void {
    const item = items.value.find((item) => item.id === id)
    if (!item) return
    item.attempts++
    item.streak = clean ? item.streak + 1 : 0
    item.dueAt =
      Date.now() + (clean ? Math.min(30, 2 ** (item.streak - 1)) * 86_400_000 : 10 * 60_000)
  }
  return { items, error, add, puzzle, answered }
})
