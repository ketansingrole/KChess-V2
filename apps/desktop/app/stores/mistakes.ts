import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { Puzzle, StoredReview } from '@kchess/contracts/types'
import type { LibrarySnapshot, MistakeExercise, MistakeGame } from '@kchess/rules/library'
import { initialLibrary } from '../utils/library'

/**
 * Answered right this many times in a row, a position is next due in four days or more (the core
 * doubles the wait from a day with each right answer): learned rather than still learning.
 */
export const LEARNED_STREAK = 3

/** Drills made from reviewed mistakes, scheduled and kept by the core. */
export const useMistakeStore = defineStore('mistakes', () => {
  const items = ref<MistakeExercise[]>(initialLibrary().mistakes)
  const error = ref('')
  /** The reviewed games the drills come from, most recently played first. */
  const games = ref<MistakeGame[]>([])
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
  async function loadGames(): Promise<void> {
    await update(window.kchess.mistakeGames(), (value) => {
      games.value = value
    })
  }
  /** Another KChess window answered or added drills. */
  function replaceFromLibrary(snapshot: LibrarySnapshot): void {
    items.value = snapshot.mistakes
  }
  return { items, error, games, add, puzzle, answered, loadGames, replaceFromLibrary }
})
