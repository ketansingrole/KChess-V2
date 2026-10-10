import { createRequire } from 'node:module'
import { logError } from './logger.ts'

/**
 * The Rust rules engine (`crates/kchess-domain` through the `@kchess/native` N-API binding).
 * Values cross the boundary as JSON text: one parse on each side is cheaper than converting
 * object graphs field by field; `null` stands for `undefined`. Services use these through
 * `services/rules.ts`.
 */
export interface NativeRules {
  /** `replaySetup`: `{ played: { uci, san }[], fen }`, or null when the setup is not legal. */
  replaySetup(variant: string, fen: string, moves: string[]): string | null
  /** `review.replay`: positions from the start as far as the moves are legal. */
  replayPositions(fen: string, moves: string[]): string
  /** `treeFromPgn`: the analysis tree, or null when the document cannot be imported without loss. */
  treeFromPgn(pgn: string): string | null
  /** Whether `treeFromPgn` accepts the document, without building the tree. */
  validPgn(pgn: string): boolean
  decodeArchivedGame(json: string): string | null
  decodeStudies(json: string): string | null
  decodeStudy(json: string): string | null
  decodeArchive(json: string): string | null
  decodeMistakes(json: string): string | null
  /** `analyseReview` / summary; null for a review the core could not have written. */
  analyseReview(json: string): string | null
  summarizeReview(json: string): string | null
  /** `lichessLine`: `{ fen, moves }`. */
  lichessLine(moves: string, pgn: string | null, initialFen: string | null): string
  sanToUci(fen: string, sans: string[]): string[]
  /** `studyDocumentPgn` over `[name, pgn]` chapters; null when a chapter holds no game. */
  studyDocumentPgn(chapters: string[][]): string | null
  studyContent(pgn: string): string
  /** Whether a study still matches its downloaded copy; null when a chapter holds no game. */
  studyMatchesCloud(chapters: string[][], downloaded: string): boolean | null
  PuzzleSampler: new (seed: number) => NativePuzzleSampler
  /** The `Math.exp` the review rules use (compared with V8's in tests). */
  jsExp(x: number): number
  /** Any rules method by name (`crates/kchess-wasm/js/engine.ts`). */
  invoke(method: string, args: string): string
  version(): string
  /** The core's services (`crates/kchess-node/src/core.rs`); see `nativeCore.ts`. */
  NativeCore: new (
    options: {
      dataDir: string
      legacyDatabasePath?: string
      bundledEnginePath?: string
      nodePath?: string
      nodeEnv?: Record<string, string>
      managedEngineDir?: string
    },
    log: (json: string) => void,
    emit: (json: string) => void,
  ) => NativeCoreHandle
  /** The offline voice model cache (`crates/kchess-node/src/voice.rs`). */
  NativeVoiceModel: new (
    directory: string,
    options: { archivePath?: string },
    log: (json: string) => void,
  ) => NativeVoiceModelHandle
  /** `VOICE_MODEL` as JSON: `{ name, url, sha256 }`. */
  voiceModelMetadata(): string
}

/** The voice model cache: status and progress are JSON (`VoiceModelStatus`, `VoiceModelProgress`). */
export interface NativeVoiceModelHandle {
  readonly path: string
  status(): Promise<string>
  ensure(progress: (json: string) => void): Promise<string>
}

/** One instance of the Rust core's services. */
export interface NativeCoreHandle {
  /** Rejects with the core's message; cancellations start with `AbortError: `. */
  call(method: string, args: string): Promise<string>
  /** Synchronous methods (storage); throws the core's message. */
  callSync(method: string, args: string): string
  close(): Promise<void>
}

/** The puzzle CSV sampler: push decompressed bytes, `finish`, then `kept()` as JSON. */
export interface NativePuzzleSampler {
  push(chunk: Uint8Array): void
  finish(): void
  readonly lines: number
  readonly count: number
  kept(): string
  /** Free the sample; the counts stay. */
  release(): void
}

let loaded: NativeRules | null | undefined

/** Which rules this process runs, for diagnostics. */
export function rulesEngine(): string {
  const rules = nativeRules()
  return rules ? `native ${rules.version()}` : 'unavailable'
}

/** The native rules, or undefined when they are not built for this host. */
export function nativeRules(): NativeRules | undefined {
  if (loaded !== undefined) return loaded ?? undefined
  loaded = null
  try {
    loaded = createRequire(import.meta.url)('@kchess/native/binding') as NativeRules
  } catch (cause) {
    logError('native', 'Native rules could not be loaded:', cause)
  }
  return loaded ?? undefined
}
