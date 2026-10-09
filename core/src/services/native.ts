import { createRequire } from 'node:module'
import { logError } from './logger'

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
  /** Any rules method by name (`core/src/domain/engine.ts`). */
  invoke(method: string, args: string): string
  version(): string
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
    loaded = createRequire(import.meta.url)('@kchess/native') as NativeRules
  } catch (cause) {
    logError('native', 'Native rules could not be loaded:', cause)
  }
  return loaded ?? undefined
}
