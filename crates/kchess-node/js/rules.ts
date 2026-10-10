import type {
  ArchivedGame,
  SavedStudy,
  StudyCommand,
  StudyCommandInput,
} from '@kchess/rules/library'
import type { ReplayedPosition } from '@kchess/rules/review'
import { setRulesBinding } from '@kchess/rules/engine'
import { logWarn } from './logger'
import { nativeRules, type NativeRules } from './native'

/**
 * The core's chess and document rules, implemented in Rust (`crates/kchess-domain`) and loaded
 * through `@kchess/native`. Services call these, never the native module directly. Rules the
 * renderer also needs still have TypeScript twins in `crates/kchess-wasm/js`;
 * `tests/core/native-rules.test.ts` keeps both identical and pins the Rust-only rules to
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

/**
 * The domain rules (`crates/kchess-wasm/js/engine.ts`) run on the native module wherever the core
 * runs: the Electron main process, the Node host and the CLI. The module loads on first use.
 */
function installRules(): void {
  // Load now, not on first use: the module is chosen by platform and architecture, which tests
  // may stub later.
  const native = rules()
  setRulesBinding({ invoke: (method, args) => native.invoke(method, args) })
}
installRules()

const parsed = <T>(json: string | null): T | undefined =>
  json === null ? undefined : (JSON.parse(json) as T)

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
