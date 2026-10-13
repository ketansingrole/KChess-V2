import { ref } from 'vue'
import type { Ref } from 'vue'
import { useSwipeBack } from './useSwipeBack'
import { useWatchStore } from '../stores/watch'

export type WatchTab = 'tv' | 'friends' | 'broadcasts'

export interface SwipeRedo {
  tourId: string
  gameId: string
}

/** Broadcast drill-down navigation: selecting, closing and leaving, with swipe redo. */
export function useBroadcastNav(tab: Ref<WatchTab>) {
  const watcher = useWatchStore()

  /** What the last swipe-back undid, while nothing else changed since. */
  const swipeRedo = ref<SwipeRedo | null>(null)
  /** Manual drill actions drop the swipe's redo; only a swipe-back sets it. */
  function selectBroadcastGame(id: string): void {
    swipeRedo.value = null
    watcher.selectedGame = id
  }
  function closeBroadcastGame(): void {
    swipeRedo.value = null
    watcher.selectedGame = ''
  }
  function leaveTour(): void {
    swipeRedo.value = null
    watcher.closeTour()
  }
  /** A swipe steps out of the broadcasts drill-down (game, tour) before leaving the page. */
  useSwipeBack({
    canBack: () =>
      tab.value === 'broadcasts' && (watcher.selectedGame !== '' || watcher.tour !== null),
    back: () => {
      if (tab.value !== 'broadcasts') return
      if (watcher.selectedGame !== '') {
        swipeRedo.value = { tourId: watcher.tour?.id ?? '', gameId: watcher.selectedGame }
        watcher.selectedGame = ''
      } else if (watcher.tour) {
        swipeRedo.value = { tourId: watcher.tour.id, gameId: '' }
        watcher.closeTour()
      }
    },
    canForward: () => {
      const redo = swipeRedo.value
      if (!redo || tab.value !== 'broadcasts') return false
      if (redo.gameId !== '') {
        // The game view closed but its tour is still open and listing the game.
        return (
          watcher.tour?.id === redo.tourId &&
          watcher.selectedGame === '' &&
          watcher.roundGames.some((entry) => entry.id === redo.gameId)
        )
      }
      return watcher.tour === null
    },
    forward: () => {
      const redo = swipeRedo.value
      swipeRedo.value = null
      if (!redo) return
      if (redo.gameId !== '' && watcher.tour?.id === redo.tourId) watcher.selectedGame = redo.gameId
      else if (redo.tourId !== '') void watcher.openTour(redo.tourId)
    },
  })

  return { swipeRedo, selectBroadcastGame, closeBroadcastGame, leaveTour }
}

export type BroadcastNav = ReturnType<typeof useBroadcastNav>
