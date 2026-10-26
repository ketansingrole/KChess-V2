/**
 * Stockfish ships with the app as the `stockfish` npm package's lite
 * multi-threaded WASM build, run as a UCI process by the host's own Node
 * runtime. A native executable picked in Settings takes precedence. The Rust core runs the
 * engine (`crates/kchess-core/src/engine`).
 */
export const BUNDLED_ENGINE_SCRIPT = 'node_modules/stockfish/bin/stockfish-19-lite.js'

/**
 * The speech recognition helper inside this package, with libvosk beside it (staged by
 * `pnpm run build:native`); the core runs it for voice input (`crates/kchess-voice`).
 */
export const VOICE_HELPER_FILE = `voice/kchess-voice${process.platform === 'win32' ? '.exe' : ''}`
/** `VOICE_HELPER_FILE` from the desktop app's directory. */
export const VOICE_HELPER = `node_modules/@kchess/native/${VOICE_HELPER_FILE}`
/** The shared core service (`docs/shared-core.md`), staged by `build:native`. */
export const SERVICE_BINARY = `node_modules/@kchess/native/service/kchess-service${
  process.platform === 'win32' ? '.exe' : ''
}`

/** A search ended on purpose (`SearchCancelled`); `isExpectedCancellation` recognises it. */
export class SearchCancelled extends Error {
  constructor() {
    super('Engine search cancelled.')
    this.name = 'SearchCancelled'
  }
}
