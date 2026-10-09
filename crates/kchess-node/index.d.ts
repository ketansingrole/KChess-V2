/** The Rust rules; JSON text crosses the boundary and `null` stands for `undefined`. */
export function version(): string
export function replaySetup(variant: string, fen: string, moves: string[]): string | null
export function replayPositions(fen: string, moves: string[]): string
export function treeFromPgn(pgn: string): string | null
export function validPgn(pgn: string): boolean
export function decodeArchivedGame(json: string): string | null
export function decodeStudies(json: string): string | null
export function decodeStudy(json: string): string | null
export function decodeArchive(json: string): string | null
export function decodeMistakes(json: string): string | null
export function analyseReview(json: string): string | null
export function summarizeReview(json: string): string | null
export function lichessLine(moves: string, pgn: string | null, initialFen: string | null): string
export function sanToUci(fen: string, sans: string[]): string[]
export function studyDocumentPgn(chapters: string[][]): string | null
export function studyContent(pgn: string): string
export function studyMatchesCloud(chapters: string[][], downloaded: string): boolean | null
export class PuzzleSampler {
  constructor(seed: number)
  push(chunk: Uint8Array): void
  finish(): void
  readonly lines: number
  readonly count: number
  kept(): string
  release(): void
}
export function jsExp(x: number): number
export function invoke(method: string, args: string): string
