import { randomInt } from 'node:crypto'
import type { ReviewSummary, StoredReview } from '../contracts/types'
import type {
  ArchivedGame,
  MistakeExercise,
  SavedStudy,
  StudyCommand,
  StudyCommandInput,
} from '../domain/library'
import type { DbPuzzle } from '../domain/puzzle'
import type { GameAnalysis, ReplayedPosition } from '../domain/review'
import { logWarn } from './logger'
import { nativeRules, type NativeRules } from './native'
import type { ChunkSampler } from './puzzleSampler'

/**
 * The core's chess and document rules, implemented in Rust (`crates/kchess-domain`) and loaded
 * through `@kchess/native`. Services call these, never the native module directly. Rules the
 * renderer also needs still have TypeScript twins in `core/src/domain`;
 * `core/tests/unit/native-rules.test.ts` keeps both identical and pins the Rust-only rules to
 * golden outputs recorded from the TypeScript rules they replaced.
 */
function rules(): NativeRules {
  const loaded = nativeRules()
  if (!loaded)
    throw new Error(
      'The KChess native rules are not built for this platform. Run `pnpm run build:native`.',
    )
  return loaded
}

const parsed = <T>(json: string | null): T | undefined =>
  json === null ? undefined : (JSON.parse(json) as T)

/** Report stored text that is not JSON at all; the decoders only say it was rejected. */
function rejected(scope: string, json: string): undefined {
  try {
    JSON.parse(json)
  } catch (cause) {
    logWarn('rules', 'Stored document is not valid JSON:', `document=${scope}`, cause)
  }
  return undefined
}

/* ── Games ── */

/** `review.replay`: the positions of a game under standard rules, as far as its moves are legal. */
export function replayPositions(fen: string, moves: readonly string[]): ReplayedPosition[] {
  return JSON.parse(rules().replayPositions(fen, [...moves])) as ReplayedPosition[]
}

/** A synced game's start position and its SAN moves as UCI. */
export function lichessGameLine(game: {
  moves: string
  pgn?: string | null
  initialFen?: string
}): { fen: string; moves: string[] } {
  return JSON.parse(rules().lichessLine(game.moves, game.pgn ?? null, game.initialFen ?? null)) as {
    fen: string
    moves: string[]
  }
}

/** SAN moves from `fen` as UCI, as far as they are legal. */
export function sanLineToUci(fen: string, sans: readonly string[]): string[] {
  return rules().sanToUci(fen, [...sans])
}

/* ── Library documents ── */

/** Whether a PGN is one bounded document that imports without loss. */
export function validPgn(pgn: string): boolean {
  return rules().validPgn(pgn)
}

/** One stored archived game, validated by replaying its moves. */
export function decodeArchivedGameText(json: string): ArchivedGame | undefined {
  return parsed<ArchivedGame>(rules().decodeArchivedGame(json)) ?? rejected('archived game', json)
}

/** The one-document game archive of earlier releases; any invalid game rejects it. */
export function decodeArchiveText(json: string): ArchivedGame[] | undefined {
  return parsed<ArchivedGame[]>(rules().decodeArchive(json)) ?? rejected('archive', json)
}

/** The stored study library; invalid studies are left out. */
export function decodeStudiesText(json: string): SavedStudy[] | undefined {
  return parsed<SavedStudy[]>(rules().decodeStudies(json)) ?? rejected('studies', json)
}

/** The stored mistake drills; each solution must be playable from its position. */
export function decodeMistakesText(json: string): MistakeExercise[] | undefined {
  return parsed<MistakeExercise[]>(rules().decodeMistakes(json)) ?? rejected('mistakes', json)
}

/** JSON for a value a frontend sent, which the native decoders read. */
function sent(value: unknown, message: string): string {
  try {
    return JSON.stringify(value)
  } catch (cause) {
    logWarn('rules', 'A value sent to the core is not JSON:', cause)
    throw new Error(message, { cause })
  }
}

/** A game a frontend saves: every field checked and every move replayed. */
export function assertArchivedGame(value: unknown): ArchivedGame {
  const message = 'This game cannot be saved.'
  const game = parsed<ArchivedGame>(rules().decodeArchivedGame(sent(value, message)))
  if (!game) throw new Error(message)
  return game
}

/** A study command; a study to restore must have only valid chapters. */
export function assertStudyCommand(command: StudyCommandInput): StudyCommand {
  if (command.op !== 'restore') return command
  const message = 'That study cannot be restored.'
  const study = parsed<SavedStudy>(rules().decodeStudy(sent(command.study, message)))
  if (!study) throw new Error(message)
  return { op: 'restore', study }
}

/** Whether a study's moves and annotations still match the copy downloaded from Lichess. */
export function studyMatchesCloud(
  study: Pick<SavedStudy, 'chapters'>,
  downloaded: string,
): boolean {
  const matches = rules().studyMatchesCloud(
    study.chapters.map((c) => [c.name, c.pgn]),
    downloaded,
  )
  // Saved chapters are valid PGN, so each holds a game.
  if (matches === null) throw new Error('A study chapter holds no game.')
  return matches
}

/* ── Reviews ── */

/** The fields the review analysis reads: scores without engine lines, most of a review's size. */
function reviewInput(review: StoredReview): string {
  return JSON.stringify({
    key: review.key,
    source: review.source,
    complete: review.complete,
    fen: review.fen,
    moves: review.moves,
    evals: review.evals.map((e) => (e && typeof e === 'object' ? { cp: e.cp, mate: e.mate } : e)),
    judgments: review.judgments,
    accuracy: review.accuracy,
  })
}

/** Reviews are only written by the core, from legal games; anything else is a bug. */
function reviewed<T>(json: string | null): T {
  if (json === null) throw new Error('This review cannot be analysed.')
  return JSON.parse(json) as T
}

/** `analyseReview`: labels, accuracy and chances for every move of a review. */
export function analyseStoredReview(review: StoredReview): GameAnalysis {
  return reviewed<GameAnalysis>(rules().analyseReview(reviewInput(review)))
}

/** What the game list shows for a review. */
export function summarizeReview(review: StoredReview): ReviewSummary {
  return reviewed<ReviewSummary>(rules().summarizeReview(reviewInput(review)))
}

/* ── Puzzle database ── */

/** The sampler for a puzzle database download. */
export function createPuzzleSampler(): ChunkSampler {
  const sampler = new (rules().PuzzleSampler)(randomInt(2 ** 32))
  return {
    push: (chunk) => sampler.push(chunk),
    finish: () => sampler.finish(),
    get count() {
      return sampler.count
    },
    get lines() {
      return sampler.lines
    },
    kept: () => {
      const rows = JSON.parse(sampler.kept()) as DbPuzzle[]
      // The worker outlives the download; free the sample rather than wait for a collection.
      sampler.release()
      return rows
    },
  }
}
