/**
 * Watching Lichess: TV channels, a game, a broadcast round, and the broadcasts Lichess features.
 * The Rust core follows the feeds (`lichess/watch.rs`) and reports frames, states and broadcast
 * updates as `watch:frame`, `watch:state` and `watch:broadcast`; the spectator forwards them.
 */
import type {
  BroadcastSummary,
  BroadcastTourDetail,
  BroadcastUpdate,
  TvChannel,
  WatchFrame,
  WatchState,
} from '../contracts/types'
import { nativeCall, nativeCallSync, onNativeEvent } from './nativeCore'

export class Spectator {
  private readonly unsubscribe: Array<() => void>

  constructor(
    frame: (frame: WatchFrame) => void,
    broadcast: (update: BroadcastUpdate) => void,
    state: (state: WatchState) => void = () => {},
  ) {
    this.unsubscribe = [
      onNativeEvent<WatchFrame>('watch:frame', frame),
      onNativeEvent<BroadcastUpdate>('watch:broadcast', broadcast),
      onNativeEvent<WatchState>('watch:state', state),
    ]
  }

  /** Stops the running watch; its frames and states are no longer reported. */
  stop(): void {
    nativeCallSync('spectate.stop')
  }

  /** Watches a TV channel (`{ channel }`) or a game (`{ gameId }`); answers the watch's session. */
  watch(target: { channel: string } | { gameId: string }): Promise<number> {
    return nativeCall('spectate.watch', target)
  }

  /** Follows a broadcast round's live PGN; answers the watch's session. */
  watchRound(roundId: string): Promise<number> {
    return nativeCall('spectate.watchRound', roundId)
  }

  /** Ends the subscriptions of this spectator; its watch is stopped first. */
  close(): void {
    this.stop()
    for (const stop of this.unsubscribe.splice(0)) stop()
  }
}

export function tvChannels(): Promise<TvChannel[]> {
  return nativeCall('spectate.tvChannels')
}

/** The broadcasts Lichess features (or a search for `query`): live, upcoming, then recently finished. */
export function broadcasts(query?: string): Promise<BroadcastSummary[]> {
  return nativeCall('spectate.broadcasts', query ?? null)
}

export function broadcastTour(id: string): Promise<BroadcastTourDetail> {
  return nativeCall('spectate.broadcastTour', id)
}
