import { describe, expect, it } from 'vitest'
import {
  addMove,
  deleteAt,
  formatEval,
  lineEnd,
  moveGlyph,
  moveNumber,
  MOVE_GLYPHS,
  movesOf,
  nodeAt,
  newTree,
  onMainline,
  pathOf,
  promoteToMainline,
  setMoveGlyph,
  treeFromPgn,
  winningChances,
  type TreeNode,
} from '../../src/domain/analysisTree'
import {
  isReconnect,
  PuzzleSession,
  puzzleSessionState,
  type PuzzleSelection,
} from '../../src/domain/puzzleSession'
import type { EngineLine, Puzzle } from '../../src/contracts/types'
import { nativeRules } from '../../src/services/native'
import { goldenFile } from './golden'

/**
 * The trainer rules (`crates/kchess-domain/src/trainer.rs`): puzzle sessions driven through
 * scripted hosts, and analysis-tree edits. The digests were recorded from the TypeScript
 * implementations the Rust rules replaced (`KCHESS_WRITE_GOLDEN=1`). Each case is generated from a
 * seed; the tests use only the public API, so they check whatever implementation is current.
 */

const loaded = nativeRules()
if (!loaded) throw new Error('The native rules are not built (pnpm run build:native).')
const native = loaded
const golden = goldenFile('trainer')

const TIMEOUT = 120_000

function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}
const pick = <T>(random: () => number, items: readonly T[]): T =>
  items[Math.floor(random() * items.length)]!

/** One rules call in the Node runtime. */
function rust(method: string, ...args: unknown[]): unknown {
  return JSON.parse(native.invoke(method, JSON.stringify(args)))
}

/** Comparable form: `undefined` is absent, non-finite numbers are their text. */
const norm = (value: unknown) =>
  value === undefined
    ? null
    : JSON.parse(
        JSON.stringify(value, (_key, inner: unknown) =>
          typeof inner === 'number' && !Number.isFinite(inner) ? String(inner) : inner,
        ),
      )

/** What a call returns or throws, as a comparable value. */
function attempt(call: () => unknown): unknown {
  try {
    return norm(call())
  } catch (cause) {
    return { error: (cause as Error).message }
  }
}

const counters = new Map<string, number>()
function check(suite: string, value: unknown): void {
  const index = counters.get(suite) ?? 0
  counters.set(suite, index + 1)
  golden.check(suite, index, norm(value))
}

/* ── Puzzle sessions ──────────────────────────────────────────────────────────────────────── */

const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'
const ERRORS = [
  'Download the puzzle database first.',
  'Lichess is unreachable.',
  'No puzzle of that theme and difficulty in the local database.',
  'late network failure',
]

interface Pending {
  method: string
  args: unknown
  resolve(value: unknown): void
  reject(cause: unknown): void
}

const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0))

function puzzleOf(random: () => number, n: number): Puzzle {
  return {
    id: `p${n}`,
    fen: START,
    solution: ['e2e4', 'e7e5'],
    rating: 1200 + Math.floor(random() * 800),
    themes: ['mix'],
    plays: Math.floor(random() * 1000),
  }
}

/** One seeded run of a puzzle session against a scripted host; every host call is logged. */
async function puzzleTrace(seed: number): Promise<unknown[]> {
  const random = rng(seed)
  const trace: unknown[] = []
  const log: string[] = []
  const pending: Pending[] = []
  let puzzles = 0
  const call =
    (method: string) =>
    (args: unknown): Promise<unknown> =>
      new Promise((resolve, reject) => {
        log.push(`${method} ${JSON.stringify(args)}`)
        pending.push({ method, args, resolve, reject })
      })
  const api = {
    localPuzzles: call('localPuzzles'),
    puzzleNext: call('puzzleNext'),
    puzzleSolve: call('puzzleSolve'),
  }
  const selection: PuzzleSelection = {
    account: random() < 0.8 ? 'Alice' : '',
    angle: pick(random, ['mix', 'fork', 'pin']),
    difficulty: pick(random, ['easiest', 'normal', 'harder']),
    color: pick(random, ['random', 'white', 'black']),
    mode: pick(random, ['rated', 'practice', 'offline']),
  }
  let online = random() < 0.8
  const state = puzzleSessionState()
  const session = new PuzzleSession(state, {
    api: api as never,
    selection: () => selection,
    online: () => online,
  })
  const settles: Promise<unknown>[] = []
  const record = (op: string) =>
    trace.push([op, norm(state), session.syncsToLichess, log.splice(0)])

  const valueFor = (method: string, kind: string): unknown => {
    if (method === 'localPuzzles') return random() < 0.7 ? [puzzleOf(random, puzzles++)] : []
    if (method === 'puzzleNext') {
      if (kind === 'reconnect') return { needsReconnect: true }
      const puzzle = puzzleOf(random, puzzles++)
      if (random() < 0.5) return { puzzle }
      return { puzzle, glicko: { rating: 1400 + Math.floor(random() * 40) / 2 } }
    }
    if (kind === 'reconnect') return { needsReconnect: true }
    return random() < 0.7 ? { ratingDiff: Math.floor(random() * 61) - 30 } : {}
  }

  for (let step = 0; step < 60; step++) {
    const r = random()
    if (r < 0.14) {
      settles.push(session.loadNext())
      record('loadNext')
    } else if (r < 0.24) {
      settles.push(session.report(random() < 0.5))
      record('report')
    } else if (r < 0.28) {
      const puzzle = state.current ?? puzzleOf(random, puzzles++)
      session.retry(puzzle)
      record('retry')
    } else if (r < 0.31) {
      session.show(puzzleOf(random, puzzles++))
      record('show')
    } else if (r < 0.34) {
      session.invalidate()
      record('invalidate')
    } else if (r < 0.37) {
      session.resetSession()
      record('resetSession')
    } else if (r < 0.46) {
      const key = pick(random, ['account', 'angle', 'difficulty', 'color', 'mode'] as const)
      if (key === 'account') selection.account = pick(random, ['Alice', 'Bob', ''])
      else if (key === 'angle') selection.angle = pick(random, ['mix', 'fork'])
      else if (key === 'difficulty')
        selection.difficulty = pick(random, ['easiest', 'easier', 'normal', 'harder', 'hardest'])
      else if (key === 'color') selection.color = pick(random, ['random', 'white', 'black'])
      else selection.mode = pick(random, ['rated', 'practice', 'offline'])
      record('select')
    } else if (r < 0.48) {
      online = !online
      record('online')
    } else if (r < 0.9 && pending.length) {
      const index =
        random() < 0.4
          ? 0
          : random() < 0.5
            ? pending.length - 1
            : Math.floor(random() * pending.length)
      const [item] = pending.splice(index, 1)!
      if (random() < 0.1) item.reject(new Error(pick(random, ERRORS)))
      else item.resolve(valueFor(item.method, random() < 0.2 ? 'reconnect' : 'ok'))
      record('settle')
    } else {
      record('idle')
    }
    await flush()
    if (step % 10 === 9) trace.push(['flushed', log.splice(0)])
  }
  // Drain everything still in flight, oldest first, so the trace ends settled.
  while (pending.length) {
    const item = pending.shift()!
    item.resolve(valueFor(item.method, 'ok'))
    await flush()
  }
  await flush()
  record('drained')
  trace.push(['settled', await Promise.allSettled(settles).then((all) => all.map((s) => s.status))])
  return trace
}

/* ── Analysis trees ───────────────────────────────────────────────────────────────────────── */

const LETTER: Record<string, string> = { knight: 'n', bishop: 'b', rook: 'r', queen: 'q' }
interface LegalMove {
  from: string
  to: string
  role: string
  promotion?: string
}
function legalUcis(fen: string): string[] {
  const moves = rust('legalMoves', { variant: 'standard', fen }) as LegalMove[]
  return moves.map((m) => `${m.from}${m.to}${m.promotion ? LETTER[m.promotion] : ''}`)
}

/** Every path in the tree, root first (the test's own walk, independent of the module). */
function pathsOf(root: TreeNode): string[] {
  const out = ['']
  const walk = (node: TreeNode, path: string): void => {
    for (const child of node.children) {
      const next = path ? `${path} ${child.uci}` : child.uci
      out.push(next)
      walk(child, next)
    }
  }
  walk(root, '')
  return out
}

/** The node at an exact path, or undefined (the test's own walk). */
function exact(root: TreeNode, path: string): TreeNode | undefined {
  let node: TreeNode | undefined = root
  for (const uci of path ? path.split(' ') : []) {
    node = node?.children.find((child) => child.uci === uci)
  }
  return node
}

const PGN_SAMPLES = [
  '1. e4 e5 2. Nf3 (2. f4 exf4) 2... Nc6 3. Bc4 *',
  '1. d4 d5 (1... Nf6 2. c4 e6) 2. c4 dxc4 *',
  '[Event "Edited"]\n\n1. e4 {a comment} e5 $1 2. Nf3 $10 Nc6 *',
]

/** One seeded run of edits on an analysis tree; the edited trees are recorded after every step. */
function treeTrace(seed: number): unknown[] {
  const random = rng(seed)
  const start =
    random() < 0.4
      ? (treeFromPgn(pick(random, PGN_SAMPLES)) ?? newTree())
      : newTree(random() < 0.1 ? 'not a fen' : START)
  const root = start
  const trace: unknown[] = []
  const snapshot = (op: string, args: unknown, result: unknown) =>
    trace.push([op, norm(args), norm(result), norm(root)])
  for (let step = 0; step < 40; step++) {
    const before = new Map(pathsOf(root).map((p) => [p, exact(root, p)!] as const))
    const paths = pathsOf(root)
    let path = pick(random, paths)
    if (random() < 0.05) path = `${path} e7e5`
    if (random() < 0.03) path = `${path}  ${pick(random, paths)}`
    const r = random()
    if (r < 0.34) {
      const node = nodeAt(root, path)
      const legal = legalUcis(node.fen)
      const uci = random() < 0.08 ? 'e2e5' : legal.length ? pick(random, legal) : 'e2e4'
      snapshot('addMove', [path, uci], addMove(root, path, uci))
    } else if (r < 0.42) {
      snapshot('deleteAt', [path], deleteAt(root, path))
    } else if (r < 0.52) {
      promoteToMainline(root, path)
      snapshot('promoteToMainline', [path], null)
    } else if (r < 0.6) {
      const node = nodeAt(root, path)
      if (random() < 0.3) node.nags = [10, pick(random, [2, 1, 6])]
      const nag = pick(random, [undefined, 0, 1, 2, 3, 4, 5, 6, 7, 10])
      setMoveGlyph(node, nag)
      snapshot('setMoveGlyph', [path, nag], node.nags)
    } else if (r < 0.66) {
      snapshot('lineEnd', [path], lineEnd(root, path))
    } else if (r < 0.74) {
      snapshot('onMainline', [path], onMainline(root, path))
    } else if (r < 0.8) {
      const node = nodeAt(root, path)
      snapshot('moveGlyph', [path], moveGlyph(node))
    } else if (r < 0.86) {
      snapshot('nodeAt', [path], { uci: nodeAt(root, path).uci, ply: nodeAt(root, path).ply })
    } else {
      snapshot('pathOf', [path], pathOf(movesOf(path)))
    }
    // Edits mutate in place: every node that survives keeps its identity.
    expect(root).toBe(start)
    for (const [p, node] of before) {
      const now = exact(root, p)
      if (now) expect(now, `node at "${p}" keeps its identity`).toBe(node)
    }
  }
  return trace
}

describe('the trainer rules match recorded traces', { timeout: TIMEOUT }, () => {
  it('runs puzzle sessions through scripted hosts (seed 8101)', async () => {
    for (let i = 0; i < 40; i++) check('puzzleSession', await puzzleTrace(8101 + i))
  })

  it('reports the initial and idle states and the predicates (seed 8102)', () => {
    check('puzzleSessionState', puzzleSessionState())
    // Hosts destructure the state's keys (the store's toRefs), so an unset rating is still a key.
    expect(Object.keys(puzzleSessionState())).toContain('rating')
    const random = rng(8102)
    const values: unknown[] = [
      null,
      undefined,
      0,
      'needsReconnect',
      [],
      {},
      { needsReconnect: false },
      { needsReconnect: true },
      { puzzle: { id: 'x' } },
      [{ needsReconnect: true }],
    ]
    for (let i = 0; i < 200; i++) values.push({ needsReconnect: random() < 0.5, id: i })
    check(
      'isReconnect',
      values.map((value) => isReconnect(value)),
    )
  })

  it('keeps the shown puzzle object when its verdict is recorded', async () => {
    // Boards reset when their puzzle object is replaced, so a verdict must not replace it.
    const puzzle = puzzleOf(rng(1), 1)
    const state = puzzleSessionState()
    const session = new PuzzleSession(state, {
      api: {
        localPuzzles: async () => [puzzle],
        puzzleNext: async () => ({ puzzle }),
        puzzleSolve: async () => ({}),
      } as never,
      selection: () => ({
        account: '',
        angle: 'mix',
        difficulty: 'normal',
        color: 'random',
        mode: 'offline',
      }),
      online: () => true,
    })
    await session.loadNext()
    const shown = state.current
    expect(shown).toEqual(puzzle)
    await session.report(false)
    expect(state.current).toBe(shown)
    expect(state.attemptOutcome).toBe(false)
    expect(state.session.failed).toBe(1)
  })

  it('edits analysis trees in place (seed 8103)', () => {
    for (let i = 0; i < 120; i++) check('treeEdits', treeTrace(8103 + i))
  })

  it('computes the glyph, number, evaluation and path helpers (seed 8104)', () => {
    const random = rng(8104)
    check(
      'moveGlyph',
      Array.from({ length: 300 }, () => {
        const nags = [undefined, [], [1], [2, 1], [10, 6], [7], [4, 3, 5]][Math.floor(random() * 7)]
        return moveGlyph({ nags: nags as number[] | undefined })
      }),
    )
    check('moveGlyphTable', MOVE_GLYPHS)
    check(
      'setMoveGlyph',
      Array.from({ length: 300 }, () => {
        const node = { nags: random() < 0.5 ? [10, 2] : random() < 0.5 ? [3, 14] : undefined }
        const nag = pick(random, [undefined, 0, 1, 2, 3, 4, 5, 6, 7, 10, 12])
        setMoveGlyph(node as TreeNode, nag)
        return node.nags
      }),
    )
    check(
      'moveNumber',
      Array.from({ length: 300 }, (_, ply) => [moveNumber(ply, false), moveNumber(ply, true)]),
    )
    const lines: (Pick<EngineLine, 'cp' | 'mate'> | undefined)[] = [undefined]
    for (let i = 0; i < 400; i++) {
      const kind = Math.floor(random() * 4)
      if (kind === 0) lines.push({ cp: Math.round((random() - 0.5) * 3000) })
      else if (kind === 1) lines.push({ cp: Math.round((random() - 0.5) * 60) * 5 })
      else if (kind === 2) lines.push({ mate: Math.floor((random() - 0.5) * 12) })
      else lines.push({ cp: Math.floor(random() * 2000) - 1000 + 0.5 })
    }
    lines.push({ mate: 0 }, {}, { cp: 25 }, { cp: -25 }, { cp: 5 }, { cp: -5 })
    check(
      'formatEval',
      lines.map((line) => formatEval(line as Pick<EngineLine, 'cp' | 'mate'>)),
    )
    check(
      'winningChances',
      lines.map((line) => winningChances(line as Pick<EngineLine, 'cp' | 'mate'>)),
    )
    const pathsSeen = ['', 'e2e4', 'e2e4 e7e5', 'e2e4  e7e5', 'e2e4 ', ' e2e4', 'a b c']
    check(
      'paths',
      pathsSeen.map((path) => [movesOf(path), pathOf(movesOf(path))]),
    )
    check('pathOfEmpty', pathOf([]))
    check(
      'attemptProbe',
      attempt(() => moveNumber(-1, true)),
    )
  })
})
