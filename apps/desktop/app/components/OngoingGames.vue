<script setup lang="ts">
import { computed } from 'vue'
import type { Key } from '@lichess-org/chessground/types'
import type { OngoingGame } from '../../../../core/src/contracts/types'
import { useChallengeStore } from '../stores/challenges'

const challenges = useChallengeStore()
const store = useKChessStore()
const { settings } = storeToRefs(store)

const games = computed(() => challenges.ongoing)
function lastMove(game: OngoingGame): Key[] | undefined {
  const move = game.lastMove
  return move && move.length >= 4 ? [move.slice(0, 2) as Key, move.slice(2, 4) as Key] : undefined
}
function timeLeft(game: OngoingGame): string {
  const seconds = game.secondsLeft
  if (seconds === undefined) return ''
  if (seconds >= 86_400)
    return `${Math.floor(seconds / 86_400)}d ${Math.floor((seconds % 86_400) / 3600)}h left`
  if (seconds >= 3600)
    return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m left`
  return `${Math.max(0, Math.floor(seconds / 60))}m left`
}
async function open(game: OngoingGame): Promise<void> {
  await store.openOngoing(game.account, game.gameId)
  store.selectPage('online')
}
</script>

<template>
  <section class="card" aria-labelledby="ongoing-title">
    <div class="card-header">
      <div>
        <h2 id="ongoing-title" class="section-title">Your games</h2>
      </div>
      <UButton
        size="xs"
        variant="ghost"
        color="neutral"
        icon="i-lucide-refresh-cw"
        aria-label="Refresh games"
        @click="challenges.refreshOngoing()"
      />
    </div>
    <p v-if="challenges.ongoingError" class="text-sm text-error" role="alert">
      {{ challenges.ongoingError }}
    </p>
    <p v-else-if="challenges.ongoingLoaded && !games.length" class="muted text-sm">
      No games in progress.
    </p>
    <div class="list-rows">
      <div v-for="game in games" :key="game.gameId" class="list-row">
        <div class="mini-board" aria-hidden="true">
          <ChessBoard
            :fen="game.fen"
            :orientation="game.color"
            :theme="settings.boardTheme"
            coordinates="none"
            :piece-set="settings.pieceSet"
            animation="none"
            :interactive="false"
            :last-move="lastMove(game)"
          />
        </div>
        <div class="row-main">
          <div class="row-title">
            <PlayerLink :username="game.opponent.name" />
            <span v-if="game.opponent.rating" class="muted tabular"
              >({{ game.opponent.rating }})</span
            >
          </div>
          <div class="row-sub">
            {{ game.speed === 'correspondence' ? 'Correspondence' : game.speed }} ·
            {{ game.rated ? 'Rated' : 'Casual' }} · you play {{ game.color }}
            <span v-if="store.connectedAccounts.length > 1"> · @{{ game.account }}</span>
          </div>
          <div class="row-sub" :class="{ 'text-primary font-semibold': game.isMyTurn }">
            {{ game.isMyTurn ? 'Your move' : 'Waiting for opponent' }}
            <span v-if="timeLeft(game)"> · {{ timeLeft(game) }}</span>
          </div>
        </div>
        <div class="row-actions">
          <UButton
            size="xs"
            :variant="game.isMyTurn ? 'solid' : 'outline'"
            :color="game.isMyTurn ? 'primary' : 'neutral'"
            :disabled="store.onlineId === game.gameId && store.onlinePhase === 'playing'"
            @click="open(game)"
            >{{
              store.onlineId === game.gameId && store.onlinePhase === 'playing' ? 'Open' : 'Play'
            }}</UButton
          >
        </div>
      </div>
    </div>
  </section>
</template>
