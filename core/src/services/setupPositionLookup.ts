/**
 * The opening explorer and master games: the Rust core looks positions up, caches them in
 * `kchess.db` and sends the signed-in account's login only to the explorer (`lichess/lookups.rs`).
 */
import type { PositionLookup } from '../contracts/types'
import { nativeCall } from './nativeCore'

export const positionLookups = {
  lookup(kind: unknown, fen: unknown, options?: unknown): Promise<PositionLookup> {
    return nativeCall('positionLookup.lookup', kind, fen, options ?? null)
  },
  mastersGame(id: unknown): Promise<string> {
    return nativeCall('positionLookup.mastersGame', id)
  },
}
