/**
 * Checks the endgame courses (`crates/kchess-domain/data/lessons.json`) with Stockfish, as the
 * drills were checked: every `play` step's goal is reached when Stockfish plays it out against
 * itself, and no move of a `line` throws away the result. The Rust tests already check that
 * every move is legal and every `mate` step has its forced mate.
 *
 *   node --experimental-strip-types tooling/verify-lessons.ts [chapter-id-prefix]
 *
 * Stockfish comes from `KCHESS_STOCKFISH`, else the managed engine in `~/.kchess`. Not part of
 * `pnpm run check`: run it after editing the lessons.
 */
import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { createInterface } from 'node:readline'
import { LESSON_COURSES, lessonStatus, type LessonStep } from '../crates/kchess-wasm/js/lessons.ts'
import { installNativeRules } from './native-rules.ts'

installNativeRules()

const binary =
  process.env.KCHESS_STOCKFISH ?? join(homedir(), '.kchess/engines/stockfish/current/stockfish')
if (!existsSync(binary)) throw new Error(`Stockfish not found at ${binary}; set KCHESS_STOCKFISH.`)

const engine = spawn(binary, [], { stdio: ['pipe', 'pipe', 'inherit'] })
const lines = createInterface({ input: engine.stdout })
let waiter: ((line: string) => boolean) | undefined
lines.on('line', (line) => {
  if (waiter?.(line)) waiter = undefined
})
const send = (command: string): void => void engine.stdin.write(`${command}\n`)
const until = (match: (line: string) => boolean): Promise<void> =>
  new Promise((resolve) => {
    waiter = (line) => {
      if (!match(line)) return false
      resolve()
      return true
    }
  })

interface Search {
  best: string
  /** Centipawns for the side to move; mates as ±(100000 - plies). */
  score: number
}

async function search(fen: string, moves: string[], movetime: number): Promise<Search> {
  let score = 0
  let best = ''
  send(`position fen ${fen}${moves.length ? ` moves ${moves.join(' ')}` : ''}`)
  const done = new Promise<void>((resolve) => {
    waiter = (line) => {
      const cp = / score cp (-?\d+)/.exec(line)
      const mate = / score mate (-?\d+)/.exec(line)
      if (cp) score = Number(cp[1])
      // `mate 0`: the side to move is mated.
      if (mate) score = (Number(mate[1]) > 0 ? 1 : -1) * (100000 - Math.abs(Number(mate[1])))
      const bestmove = /^bestmove (\S+)/.exec(line)
      if (!bestmove) return false
      best = bestmove[1]!
      resolve()
      return true
    }
  })
  send(`go movetime ${movetime}`)
  await done
  return { best, score }
}

const WON = 400
const failures: string[] = []
const fail = (at: string, problem: string): void => {
  failures.push(`${at}: ${problem}`)
  console.log(`FAIL ${at}: ${problem}`)
}

/** Plays `step` out, Stockfish on both sides, and reports the result. */
async function playOut(at: string, step: LessonStep): Promise<void> {
  const moves: string[] = []
  for (let ply = 0; ply < 260; ply++) {
    const status = lessonStatus(step.fen, step.task, moves)
    if (status.over) {
      if (!status.success) fail(at, `self-play ended ${status.reason} after ${moves.join(' ')}`)
      else console.log(`ok   ${at}: ${status.reason} in ${status.played}`)
      return
    }
    const { best } = await search(step.fen, moves, status.turn === status.player ? 700 : 250)
    moves.push(best)
  }
  fail(at, 'self-play did not finish')
}

async function checkLine(at: string, step: LessonStep, moves: string[]): Promise<void> {
  const start = await search(step.fen, [], 1500)
  const winning = start.score >= WON
  for (let i = 0; i < moves.length; i += 2) {
    // After your move the opponent is to move: their score must not have improved past the class.
    const after = await search(step.fen, moves.slice(0, i + 1), 1000)
    const ours = -after.score
    if (winning ? ours < WON / 2 : ours < -150)
      fail(at, `${moves[i]} (move ${i / 2 + 1}) scores ${ours} from ${start.score}`)
  }
  console.log(`ok   ${at}: line from ${start.score}`)
}

send('uci')
await until((line) => line === 'uciok')
send('setoption name Threads value 4')
send('setoption name Hash value 256')
send('isready')
await until((line) => line === 'readyok')

const only = process.argv[2]
for (const course of LESSON_COURSES)
  for (const chapter of course.chapters) {
    if (only && !chapter.id.startsWith(only)) continue
    for (const [index, step] of chapter.steps.entries()) {
      const at = `${chapter.id} #${index + 1}`
      if (step.task.kind === 'line') await checkLine(at, step, step.task.moves)
      else if (step.task.kind === 'play') {
        // Static scores only warn: without tablebases Stockfish misjudges some theoretical
        // wins and draws, so playing the step out is the test.
        const { score } = await search(step.fen, [], 2000)
        const goal = step.task.goal
        if (goal === 'draw' ? Math.abs(score) > 150 : score < WON)
          console.log(`warn ${at}: ${goal} position scores ${score}`)
        await playOut(at, step)
      }
    }
  }

engine.kill()
console.log(failures.length ? `\n${failures.length} problems` : '\nAll lessons verified.')
process.exitCode = failures.length ? 1 : 0
