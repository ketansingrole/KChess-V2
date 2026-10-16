import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { wasmBinding, type RulesBinding } from '@kchess/rules/engine'
import lessons from '../../crates/kchess-domain/data/lessons.json' with { type: 'json' }
import { nativeRules } from '@kchess/native/native'
import { goldenFile } from './golden'
import { capture } from './helpers/capture.ts'

/**
 * The lesson rules (`crates/kchess-domain/src/training/lessons.rs`) exist only in Rust, so their
 * outputs are pinned here (`golden/lessons.json`; rewrite with `KCHESS_WRITE_GOLDEN=1` only for
 * an intended change). Every call runs in the Node module and the WebAssembly build, which must
 * agree. Each step is walked with its own hints and replies, and with seeded random moves.
 */

const loaded = nativeRules()
if (!loaded) throw new Error('The native rules are not built (pnpm run build:native).')
const native = loaded
const WASM_PATH = join(
  dirname(fileURLToPath(import.meta.url)),
  '../../apps/desktop/app/assets/rules/kchess.wasm',
)
const wasm: RulesBinding = wasmBinding(
  new WebAssembly.Instance(new WebAssembly.Module(readFileSync(WASM_PATH))).exports,
)
const golden = goldenFile('lessons')

function rust(method: string, ...args: unknown[]) {
  const json = JSON.stringify(args)
  const fromNode = native.invoke(method, json)
  expect(wasm.invoke(method, json), `${method} in WebAssembly`).toBe(fromNode)
  return JSON.parse(fromNode)
}

function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

interface LegalMove {
  from: string
  to: string
  promotion?: string
}
const LETTER: Record<string, string> = { knight: 'n', bishop: 'b', rook: 'r', queen: 'q' }
const legal = (fen: string): string[] =>
  (rust('legalMoves', { variant: 'standard', fen }) as LegalMove[]).map(
    (m) => `${m.from}${m.to}${m.promotion ? LETTER[m.promotion] : ''}`,
  )

const steps = lessons.courses.flatMap((course) =>
  course.chapters.flatMap((chapter) => chapter.steps),
)

const counters = new Map<string, number>()
function check(suite: string, value: unknown): void {
  const index = counters.get(suite) ?? 0
  counters.set(suite, index + 1)
  golden.check(suite, index, value)
}

describe('lesson rules golden outputs', () => {
  it('walks every step with its hints and replies', () => {
    for (const step of steps) {
      const moves: string[] = []
      const trace: unknown[] = []
      for (let ply = 0; ply < 30; ply++) {
        const status = rust('lessonStatus', step.fen, step.task, moves)
        trace.push(status)
        if (status.over || status.engine) break
        const next =
          status.turn === status.player
            ? rust('lessonHint', step.fen, step.task, moves)
            : status.reply
        if (!next) break
        if (status.turn === status.player)
          trace.push(rust('lessonMove', step.fen, step.task, moves, next))
        moves.push(next)
      }
      check('walk', trace)
    }
  })

  it('judges seeded random moves in every step', () => {
    const random = rng(20261016)
    for (const step of steps) {
      const moves: string[] = []
      const trace: unknown[] = []
      for (let ply = 0; ply < 12; ply++) {
        const status = rust('lessonStatus', step.fen, step.task, moves)
        if (status.over) break
        const options = legal(status.fen)
        const uci = options[Math.floor(random() * options.length)]
        if (!uci) break
        if (status.turn === status.player)
          trace.push(rust('lessonMove', step.fen, step.task, moves, uci))
        moves.push(uci)
        trace.push(rust('lessonStatus', step.fen, step.task, moves))
      }
      check('random', trace)
    }
  })

  it('rejects malformed calls the same way', () => {
    const cases: unknown[][] = [
      ['lessonStatus'],
      ['lessonStatus', 'nonsense', { kind: 'read' }, []],
      ['lessonStatus', '8/8/8/8/8/8/8/K6k w - - 0 1', { kind: 'fly' }, []],
      ['lessonMove', '8/8/8/8/8/8/8/K6k w - - 0 1', { kind: 'read' }, [], 'a1a2'],
      ['lessonHint', '8/8/8/8/8/8/8/K6k w - - 0 1', { kind: 'mate', within: 1 }, []],
      ['lessonProblems'],
    ]
    for (const [method, ...args] of cases)
      check(
        'calls',
        capture(() => rust(method as string, ...args)),
      )
  })
})
