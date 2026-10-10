/**
 * Stockfish ships with the app as the `stockfish` npm package's lite
 * multi-threaded WASM build, run as a UCI process by the host's own Node
 * runtime. A native executable picked in Settings takes precedence. The Rust core runs the
 * engine (`crates/kchess-core/src/engine`).
 */
export const BUNDLED_ENGINE_SCRIPT = 'node_modules/stockfish/bin/stockfish-19-lite.js'

/** A search ended on purpose (`SearchCancelled`); `isExpectedCancellation` recognises it. */
export class SearchCancelled extends Error {
  constructor() {
    super('Engine search cancelled.')
    this.name = 'SearchCancelled'
  }
}
