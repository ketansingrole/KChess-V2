import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { Puzzle, StoredReview } from '@kchess/contracts/types'
import type { MistakeExercise } from '@kchess/rules/library'
import { initialLibrary } from '../utils/library'

/** Drills made from reviewed mistakes, scheduled and kept by the core. */
export const useMistakeStore = defineStore('mistakes', () => {
  const items = ref<MistakeExercise[]>(initialLibrary().mistakes)
  const error = ref('')
  async function update<T>(request: Promise<T>, apply: (value: T) => void): Promise<T> {
    try {
      const value = await request
      apply(value)
      error.value = ''
      return value
    } catch (cause) {
      console.warn('[mistakes] saving drills failed:', cause)
      error.value = cause instanceof Error ? cause.message : String(cause)
      throw cause
    }
  }
  /** Drills for a completed review's mistakes (one side's, when given); how many are new. */
  async function add(review: StoredReview, color?: 'white' | 'black'): Promise<number> {
    if (!review.complete) return 0
    const result = await update(window.kchess.addMistakes(review.key, color), (value) => {
      items.value = value.items
    })
    return result.added
  }
  function puzzle(item: MistakeExercise): Puzzle {
    return { id: item.id, fen: item.fen, solution: [...item.solution], rating: 0, themes: [] }
  }
  async function answered(id: string, clean: boolean): Promise<void> {
    await update(window.kchess.answerMistake(id, clean), (value) => {
      items.value = value
    })
  }
  return { items, error, add, puzzle, answered }
})
