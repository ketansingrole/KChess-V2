import { writeFileSync } from 'node:fs'
import { afterAll, it, vi } from 'vitest'
import { OnlineGame, onlineGameState, type OnlineGameHost } from '../../src/domain/onlineGame.ts'
import {
  ONLINE_ACTIONS,
  type ChatLine,
  type OnlineAction,
  type OnlineConnection,
  type OnlineEvent,
  type OnlineOptions,
} from '../../src/contracts/types.ts'
import { goldenFile } from './golden.ts'

/**
 * The online game (`core/src/domain/onlineGame.ts`) against its Rust transitions. Each case is a
 * sequence of user actions, server events, connection reports and network replies driven through a
 * fake host. After every step the trace records what the game exposed: host callbacks and API calls
 * in order, console warnings, the outcome of each call, the state, the clock and the derived views.
 * Digests are recorded from the TypeScript class first (`KCHESS_WRITE_GOLDEN=1`), then checked
 * against the Rust-backed class (see RUST_MIGRATION.md). `KCHESS_ONLINE_GAME_DUMP=<file>` writes the
 * full traces as JSON for diagnosis.
 */

const golden = goldenFile('online-game')
const NETWORK = [
  'onlineChat',
  'playOnline',
  'onlineAction',
  'startOnline',
  'cancelOnline',
  'resumeOnline',
  'openGame',
] as const
const IDS = ['AbCd1234', 'ZyXw9876'] as const
const STATUSES = [
  'started',
  'created',
  'mate',
  'resign',
  'aborted',
  'draw',
  'stalemate',
  'outoftime',
] as const
const MOVES = [
  'e2e4',
  'e7e5',
  'g1f3',
  'b8c6',
  'f1c4',
  'g8f6',
  'd2d4',
  'e5d4',
  'c1f4',
  'f8c5',
  'e1g1',
  'd7d5',
  'zz9z',
  'a2a4',
  'h7h5',
]
const FEN_AFTER_E4 = 'rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1'
const dumps: Record<string, unknown> = {}

type Step =
  | { op: 'event'; event: unknown }
  | { op: 'connection'; connection: unknown }
  | { op: 'checking' }
  | { op: 'start'; options: OnlineOptions }
  | { op: 'stop' }
  | { op: 'reconnect' }
  | { op: 'open'; account: string; id: string }
  | { op: 'move'; uci: string }
  | { op: 'action'; action: OnlineAction }
  | { op: 'chat'; id: string }
  | { op: 'finish'; winner?: 'white' | 'black'; reason: string; notify: boolean }
  | { op: 'reset' }
  | { op: 'settle'; which: 'first' | 'last' | number; ok: boolean; pick?: number; value?: unknown }
  | { op: 'time'; ms: number }
  | { op: 'account'; name: string }
  | { op: 'chatEnabled'; on: boolean }

interface Pending {
  method: string
  resolve(value: unknown): void
  reject(cause: unknown): void
  settled: boolean
}

function rng(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

function pick<T>(random: () => number, items: readonly T[]): T {
  return items[Math.floor(random() * items.length)] as T
}

function text(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause)
}

/** A value as the trace keeps it: JSON, with `undefined` spelled out. */
function shown(value: unknown): unknown {
  return value === undefined ? 'undefined' : JSON.parse(JSON.stringify(value) ?? 'null')
}

/** The network replies a method is given when the script does not choose one. */
function settleValue(method: string, choice = 0): unknown {
  switch (method) {
    case 'startOnline':
      return [{}, { url: 'https://lichess.org/AbCd1234' }, { seeking: true }][choice % 3]
    case 'resumeOnline':
      return [null, { id: IDS[0], account: 'Alice' }, { id: IDS[1], account: 'Bob' }][choice % 3]
    case 'onlineChat': {
      const lines: ChatLine[][] = [
        [],
        [{ user: 'Bob', text: 'gg', room: 'player' }],
        [
          { user: 'Ref', text: 'watch', room: 'spectator' },
          { user: 'Bob', text: 'hi', room: 'player' },
        ],
      ]
      return lines[choice % 3]
    }
    default:
      return undefined
  }
}

/** Runs the steps against a fresh game and returns one observation per step. */
async function play(steps: Step[], account: string): Promise<unknown[]> {
  let time = 0
  let activeAccount = account
  let chatOn = true
  const log: unknown[] = []
  const calls: Pending[] = []
  const api: Record<string, (...args: unknown[]) => Promise<unknown>> = {}
  for (const method of NETWORK)
    api[method] = (...args: unknown[]) => {
      log.push(['api', method, shown(args)])
      return new Promise((resolve, reject) => {
        calls.push({ method, resolve, reject, settled: false })
      })
    }
  const warn = vi
    .spyOn(console, 'warn')
    .mockImplementation((...parts: unknown[]) =>
      log.push(['warn', String(parts[0]), parts.length > 1 ? text(parts[1]) : 'none']),
    )
  const state = onlineGameState()
  state.onlineAccount = account
  const host: OnlineGameHost = {
    api: api as unknown as OnlineGameHost['api'],
    activeAccount: () => activeAccount,
    chatEnabled: () => chatOn,
    now: () => time,
    notify: (kind, title, body) => log.push(['notify', kind, title, body]),
    moved: (san) => log.push(['moved', String(san)]),
    resetView: () => log.push(['resetView']),
    connectionChanged: () => log.push(['connectionChanged']),
    moveAcknowledged: (ms) => log.push(['moveAcknowledged', ms]),
    failed: (cause) => log.push(['failed', text(cause)]),
  }
  const game = new OnlineGame(state, host)
  const track = (label: string, promise: Promise<unknown>) =>
    promise.then(
      (value) => log.push(['done', label, shown(value)]),
      (cause) => log.push(['done', label, 'rejected', text(cause)]),
    )
  const settle = (step: Extract<Step, { op: 'settle' }>) => {
    const unsettled = calls.filter((call) => !call.settled)
    const call =
      step.which === 'first'
        ? unsettled[0]
        : step.which === 'last'
          ? unsettled.at(-1)
          : calls[step.which]
    if (!call || call.settled) return
    call.settled = true
    if (step.ok) call.resolve(step.value ?? settleValue(call.method, step.pick))
    else call.reject(new Error(String(step.value ?? `network failed: ${call.method}`)))
  }
  const records: unknown[] = []
  try {
    for (let index = 0; index < steps.length; index++) {
      const step = steps[index]!
      try {
        switch (step.op) {
          case 'event':
            game.readOnlineEvent(step.event as OnlineEvent)
            break
          case 'connection':
            game.readOnlineState(step.connection as OnlineConnection)
            break
          case 'checking':
            game.checkingOnline()
            break
          case 'start':
            void track('start', game.start(step.options))
            break
          case 'stop':
            void track('stop', game.stop())
            break
          case 'reconnect':
            void track('reconnect', game.reconnect())
            break
          case 'open':
            void track('open', game.open(step.account, step.id))
            break
          case 'move':
            void track('move', game.onlineMove(step.uci))
            break
          case 'action':
            void track('action', game.onlineAction(step.action))
            break
          case 'chat':
            void track('chat', game.loadChat(step.id))
            break
          case 'finish':
            game.finish(step.winner, step.reason, step.notify)
            break
          case 'reset':
            game.resetGame()
            break
          case 'settle':
            settle(step)
            break
          case 'time':
            time += step.ms
            break
          case 'account':
            activeAccount = step.name
            break
          case 'chatEnabled':
            chatOn = step.on
            break
        }
      } catch (cause) {
        log.push(['threw', step.op, text(cause)])
      }
      await new Promise((resolve) => setTimeout(resolve, 0))
      const view = (read: () => unknown) => {
        try {
          return shown(read())
        } catch (cause) {
          return ['threw', text(cause)]
        }
      }
      records.push(
        shown({
          index,
          step,
          log: log.splice(0),
          state: { ...state },
          shownGame: game.shownGame,
          clock: [
            game.clock.remaining('white', time),
            game.clock.remaining('black', time),
            game.clock.running ?? null,
          ],
          view: {
            position: view(() => ({ fen: game.position.fen, turn: game.position.turn })),
            history: view(() => game.history),
            ownMoves: view(() => game.ownMoves),
            correspondence: view(() => game.correspondence),
            rematch: view(() => game.rematchOptions() ?? null),
          },
        }),
      )
    }
  } finally {
    warn.mockRestore()
  }
  return records
}

/** Random steps: events (in order, stale, for other games), connection reports and replies. */
function randomSteps(seed: number): Step[] {
  const random = rng(seed * 7919 + 1)
  const moves = (): string => {
    const count = Math.floor(random() * (MOVES.length + 1))
    return random() < 0.2
      ? `  ${MOVES.slice(0, count).join('  ')} `
      : MOVES.slice(0, count).join(' ')
  }
  const flags = (): Record<string, unknown> => {
    const out: Record<string, unknown> = {
      moves: moves(),
      status: pick(random, STATUSES),
      wtime: pick(random, [60000, 45000, 1000, 0, 120000]),
      btime: pick(random, [60000, 58000, 3000, 180000]),
    }
    if (random() < 0.2) out.wdraw = true
    if (random() < 0.2) out.bdraw = true
    if (random() < 0.15) out.wtakeback = true
    if (random() < 0.15) out.btakeback = true
    if (random() < 0.15)
      out.expiration = { idleMillis: 1000, millisToMove: pick(random, [5000, 20000]) }
    if (random() < 0.1) out.winner = pick(random, ['white', 'black'])
    return out
  }
  const player = (): Record<string, unknown> =>
    pick(random, [
      { id: 'alice', name: 'Alice', rating: 1500 },
      { id: 'bob', name: 'Bob' },
      { id: 'Alice' },
      { name: 'Stockfish', aiLevel: 3 },
      { aiLevel: 0 },
      { id: 'carol', rating: 1200 },
      {},
    ])
  const eventOf = (): unknown => {
    const id = random() < 0.8 ? IDS[0] : pick(random, IDS)
    switch (
      pick(random, [
        'full',
        'full',
        'state',
        'state',
        'state',
        'start',
        'finish',
        'gone',
        'chat',
        'challenge',
      ])
    ) {
      case 'full':
        return {
          type: 'gameFull',
          id,
          rated: random() < 0.5,
          ...(random() < 0.6 ? { speed: pick(random, ['blitz', 'correspondence']) } : {}),
          ...(random() < 0.5
            ? {
                variant: {
                  key: pick(random, [
                    'standard',
                    'chess960',
                    'crazyhouse',
                    'fromPosition',
                    'threeCheck',
                  ]),
                },
              }
            : {}),
          ...(random() < 0.4 ? { initialFen: pick(random, ['startpos', '', FEN_AFTER_E4]) } : {}),
          ...(random() < 0.7
            ? {
                clock: pick(random, [
                  { initial: 60000, increment: 1000 },
                  { initial: 180000, increment: 0 },
                  null,
                ]),
              }
            : {}),
          ...(random() < 0.2 ? { daysPerTurn: pick(random, [1, 3, 14]) } : {}),
          ...(random() < 0.2 ? { tournamentId: 'T1' } : {}),
          white: player(),
          black: player(),
          state: { type: 'gameState', ...flags() },
        }
      case 'state':
        return { type: 'gameState', ...flags(), ...(random() < 0.1 ? { id } : {}) }
      case 'start':
        return {
          type: 'gameStart',
          game: {
            gameId: id,
            ...(random() < 0.6 ? { speed: pick(random, ['blitz', 'correspondence']) } : {}),
            ...(random() < 0.7 ? { color: pick(random, ['white', 'black']) } : {}),
            ...(random() < 0.7
              ? { opponent: pick(random, [{ username: 'Bob', id: 'bob' }, {}]) }
              : {}),
            ...(random() < 0.5 ? { rated: random() < 0.5 } : {}),
          },
        }
      case 'finish':
        return {
          type: 'gameFinish',
          game: {
            gameId: id,
            ...(random() < 0.6 ? { winner: pick(random, ['white', 'black']) } : {}),
            ...(random() < 0.8 ? { status: { name: pick(random, STATUSES) } } : {}),
          },
        }
      case 'gone':
        return {
          type: 'opponentGone',
          gone: random() < 0.6,
          ...(random() < 0.6 ? { claimWinInSeconds: pick(random, [0, 15, 30]) } : {}),
          ...(random() < 0.3 ? { id } : {}),
        }
      case 'chat':
        return {
          type: 'chatLine',
          username: pick(random, ['Bob', 'alice', 'Carol']),
          text: pick(random, ['hi', 'gg', 'nice']),
          room: pick(random, ['player', 'spectator']),
          ...(random() < 0.7 ? { id } : {}),
        }
      default:
        return { type: 'challenge', challenge: { id: 'Chal1234' } }
    }
  }
  const steps: Step[] = []
  const count = 45
  for (let i = 0; i < count; i++) {
    const roll = random()
    if (roll < 0.3) steps.push({ op: 'event', event: eventOf() })
    else if (roll < 0.42)
      steps.push({
        op: 'connection',
        connection: {
          session: pick(random, [-1, 0, 1, 2, 3]),
          account: pick(random, ['Alice', 'Bob', '']),
          gameId: pick(random, [IDS[0], IDS[1], '']),
          lane: pick(random, ['events', 'game', 'seek'] as const),
          phase: pick(random, [
            'checking',
            'connecting',
            'connected',
            'reconnecting',
            'disconnected',
            'auth-required',
            'idle',
          ] as const),
          ...(random() < 0.5 ? { message: pick(random, ['Lichess is slow', 'Reconnecting']) } : {}),
        },
      })
    else if (roll < 0.45) steps.push({ op: 'checking' })
    else if (roll < 0.51)
      steps.push({
        op: 'start',
        options: {
          minutes: pick(random, [1, 3, 5, 15]),
          increment: pick(random, [0, 2]),
          color: pick(random, ['random', 'white', 'black'] as const),
          rated: random() < 0.5,
          ...(random() < 0.3 ? { days: pick(random, [1, 3, 14] as const) } : {}),
          ...(random() < 0.3 ? { target: 'bob' } : {}),
          ...(random() < 0.3 ? { account: pick(random, ['Alice', 'Bob']) } : {}),
          ...(random() < 0.1 ? { variant: 'chess960' as const } : {}),
        },
      })
    else if (roll < 0.54) steps.push({ op: 'stop' })
    else if (roll < 0.59) steps.push({ op: 'reconnect' })
    else if (roll < 0.63)
      steps.push({ op: 'open', account: pick(random, ['Alice', 'Bob']), id: pick(random, IDS) })
    else if (roll < 0.7) steps.push({ op: 'move', uci: pick(random, MOVES.slice(0, 6)) })
    else if (roll < 0.75) steps.push({ op: 'action', action: pick(random, ONLINE_ACTIONS) })
    else if (roll < 0.77) steps.push({ op: 'chat', id: pick(random, IDS) })
    else if (roll < 0.78)
      steps.push({
        op: 'finish',
        winner: pick(random, [undefined, 'white', 'black'] as const),
        reason: pick(random, ['aborted', 'resign', 'draw']),
        notify: random() < 0.7,
      })
    else if (roll < 0.79) steps.push({ op: 'reset' })
    else if (roll < 0.93)
      steps.push({
        op: 'settle',
        which: random() < 0.6 ? 'last' : random() < 0.5 ? 'first' : Math.floor(random() * 8),
        ok: random() < 0.75,
        pick: Math.floor(random() * 3),
      })
    else if (roll < 0.97) steps.push({ op: 'time', ms: pick(random, [0, 250, 2000, 9000, 20000]) })
    else if (roll < 0.985) steps.push({ op: 'account', name: pick(random, ['Alice', 'Bob', '']) })
    else steps.push({ op: 'chatEnabled', on: random() < 0.6 })
  }
  return steps
}

const full = (id = 'AbCd1234', moves = '', extra: Record<string, unknown> = {}): OnlineEvent =>
  ({
    type: 'gameFull',
    id,
    rated: false,
    initialFen: 'startpos',
    variant: { key: 'standard' },
    white: { id: 'alice', name: 'Alice', rating: 1500 },
    black: { id: 'bob', name: 'Bob', rating: 1400 },
    clock: { initial: 60000, increment: 1000 },
    speed: 'blitz',
    state: { type: 'gameState', moves, status: 'started', wtime: 60000, btime: 60000 },
    ...extra,
  }) as unknown as OnlineEvent
const live = (moves: string, extra: Record<string, unknown> = {}): OnlineEvent =>
  ({
    type: 'gameState',
    moves,
    status: 'started',
    wtime: 58000,
    btime: 59000,
    ...extra,
  }) as unknown as OnlineEvent
const start = (extra: Partial<OnlineOptions> = {}): Step => ({
  op: 'start',
  options: { minutes: 5, increment: 0, rated: false, color: 'random', ...extra },
})
const settle = (which: 'first' | 'last' | number, ok = true, value?: unknown): Step => ({
  op: 'settle',
  which,
  ok,
  value,
})
const event = (value: unknown): Step => ({ op: 'event', event: value })

const SCRIPTED: [string, Step[]][] = [
  [
    'seek, play and resign',
    [
      start(),
      settle('last', true, { url: 'https://lichess.org/AbCd1234' }),
      event({
        type: 'gameStart',
        game: {
          gameId: 'AbCd1234',
          speed: 'blitz',
          color: 'white',
          opponent: { username: 'Bob', id: 'bob' },
          rated: false,
        },
      }),
      event(full()),
      { op: 'move', uci: 'e2e4' },
      settle('last', true),
      event(live('e2e4')),
      event(live('e2e4 e7e5')),
      event({
        type: 'gameFinish',
        game: { gameId: 'AbCd1234', winner: 'black', status: { name: 'resign' } },
      }),
    ],
  ],
  [
    'start fails, then retries',
    [
      start(),
      settle('last', false, 'network down'),
      start({ rated: true }),
      settle('last', true, {}),
      { op: 'stop' },
      settle('last', true),
    ],
  ],
  [
    'start fails while a seek is cancelled',
    [start(), { op: 'stop' }, settle('first', false, 'late failure'), settle('last', true)],
  ],
  [
    'late seek reply after stop',
    [
      start(),
      { op: 'stop' },
      settle('first', true, { seeking: true }),
      settle('last', false, 'cancel failed'),
    ],
  ],
  [
    'reconnect supersedes a pending seek',
    [start(), { op: 'reconnect' }, settle('first', true, {}), settle('last', true, null)],
  ],
  [
    'reconnect resumes the game',
    [
      event(full()),
      {
        op: 'connection',
        connection: {
          session: 1,
          lane: 'game',
          account: 'Alice',
          gameId: 'AbCd1234',
          phase: 'disconnected',
          message: 'Lost',
        },
      },
      { op: 'reconnect' },
      settle('last', true, { id: 'AbCd1234', account: 'Alice' }),
      event(full('AbCd1234', 'e2e4')),
    ],
  ],
  [
    'reconnect finds nothing or fails',
    [
      { op: 'reconnect' },
      settle('last', true, null),
      { op: 'reconnect' },
      settle('last', false, 'offline'),
    ],
  ],
  [
    'open the shown game, then another one',
    [
      event(full('AbCd1234', 'e2e4')),
      { op: 'open', account: 'Alice', id: 'AbCd1234' },
      settle('last', true),
      { op: 'open', account: 'Alice', id: 'ZyXw9876' },
      settle('last', false, 'no such game'),
      { op: 'open', account: 'Bob', id: 'ZyXw9876' },
      settle('last', true),
    ],
  ],
  [
    'move acknowledgement timing',
    [
      event(full()),
      { op: 'move', uci: 'e2e4' },
      { op: 'time', ms: 80 },
      settle('last', true),
      { op: 'time', ms: 1000 },
      event(full('AbCd1234', 'e2e4')),
    ],
  ],
  [
    'an ambiguous move is never replayed',
    [
      event(full()),
      { op: 'move', uci: 'e2e4' },
      settle('last', false, 'lost response'),
      { op: 'move', uci: 'e2e4' },
      { op: 'action', action: 'resign' },
    ],
  ],
  [
    'a move acknowledged after a new session is ignored',
    [
      event(full()),
      { op: 'move', uci: 'e2e4' },
      {
        op: 'connection',
        connection: {
          session: 5,
          lane: 'game',
          account: 'Alice',
          gameId: 'AbCd1234',
          phase: 'connected',
        },
      },
      settle('last', true),
      { op: 'move', uci: 'e2e4' },
      settle('last', false, 'late'),
    ],
  ],
  [
    'draw and takeback offers, declined and accepted',
    [
      event(full()),
      { op: 'action', action: 'offerDraw' },
      settle('last', true),
      { op: 'action', action: 'declineDraw' },
      settle('last', true),
      event(live('e2e4', { bdraw: true })),
      { op: 'action', action: 'acceptDraw' },
      settle('last', true),
      event(live('e2e4', { btakeback: true })),
      { op: 'action', action: 'takeback' },
      settle('last', true),
      { op: 'action', action: 'declineTakeback' },
      settle('last', false, 'refused'),
      { op: 'action', action: 'berserk' },
      settle('last', true),
    ],
  ],
  [
    'takeback requested, then answered by a new move',
    [
      event(full()),
      event(live('e2e4', { wtakeback: true })),
      event(live('e2e4 e7e5')),
      event(live('e2e4 e7e5', { btakeback: true })),
      { op: 'action', action: 'takeback' },
      settle('last', true),
    ],
  ],
  [
    'chat loads, streams, and stops when disabled',
    [
      event(full()),
      settle('last', true, [
        { user: 'Ref', text: 'watch', room: 'spectator' },
        { user: 'Bob', text: 'hi', room: 'player' },
      ]),
      event({ type: 'chatLine', username: 'Bob', text: 'gg', room: 'player', id: 'AbCd1234' }),
      event({ type: 'chatLine', username: 'Ann', text: 'wow', room: 'spectator', id: 'AbCd1234' }),
      event({ type: 'chatLine', username: 'Ann', text: 'other', room: 'player', id: 'ZyXw9876' }),
      { op: 'chatEnabled', on: false },
      event({ type: 'chatLine', username: 'Bob', text: 'hidden', room: 'player' }),
      { op: 'chat', id: 'AbCd1234' },
    ],
  ],
  [
    'a failed chat load leaves the game running',
    [
      event(full()),
      settle('last', false, 'chat down'),
      { op: 'chat', id: 'AbCd1234' },
      settle('last', true, []),
    ],
  ],
  [
    'opponent leaves and comes back, then the win is claimed',
    [
      event(full()),
      event({ type: 'opponentGone', gone: true, claimWinInSeconds: 30, id: 'AbCd1234' }),
      { op: 'time', ms: 5000 },
      event({ type: 'opponentGone', gone: false }),
      event({ type: 'opponentGone', gone: true, claimWinInSeconds: 10 }),
      event({ type: 'opponentGone', gone: true, id: 'ZyXw9876', claimWinInSeconds: 1 }),
    ],
  ],
  [
    'first-move expiry and the clock',
    [
      event(full()),
      event(live('', { expiration: { idleMillis: 1000, millisToMove: 20000 }, wtime: 60000 })),
      { op: 'time', ms: 3000 },
      event(
        live('e2e4', {
          expiration: { idleMillis: 1000, millisToMove: 20000 },
          wtime: 50000,
          btime: 61000,
        }),
      ),
    ],
  ],
  [
    'arena berserk window',
    [
      event(full('AbCd1234', '', { tournamentId: 'T1', clock: { initial: 120000, increment: 0 } })),
      event(live('', { wtime: 60000, btime: 120000 })),
      { op: 'action', action: 'berserk' },
      settle('last', true),
      event(live('e2e4', { wtime: 60000, btime: 120000 })),
    ],
  ],
  [
    'unsupported variant and a custom start',
    [
      event(full('AbCd1234', '', { variant: { key: 'crazyhouse' } })),
      event(
        full('ZyXw9876', 'e2e4', { variant: { key: 'fromPosition' }, initialFen: FEN_AFTER_E4 }),
      ),
      event(full('AbCd1234', '', { variant: { key: 'chess960' }, initialFen: 'startpos' })),
    ],
  ],
  [
    'correspondence game and its rematch',
    [
      start({ days: 3 }),
      settle('last', true, {}),
      event(full('AbCd1234', '', { speed: 'correspondence', daysPerTurn: 3, clock: null })),
      event(live('e2e4')),
      event({
        type: 'gameFinish',
        game: { gameId: 'AbCd1234', winner: 'white', status: { name: 'mate' } },
      }),
      {
        op: 'start',
        options: { minutes: 0, increment: 0, color: 'black', rated: true, days: 14, target: 'bob' },
      },
      settle('last', true, {}),
    ],
  ],
  [
    'account switch keeps the playing account',
    [
      { op: 'account', name: 'Bob' },
      event(full()),
      {
        op: 'connection',
        connection: {
          session: 2,
          lane: 'game',
          account: 'Carol',
          gameId: 'AbCd1234',
          phase: 'reconnecting',
        },
      },
      {
        op: 'connection',
        connection: { session: 1, lane: 'game', account: 'Dan', gameId: 'Other123', phase: 'idle' },
      },
      {
        op: 'connection',
        connection: { session: 3, lane: 'game', account: '', gameId: '', phase: 'idle' },
      },
    ],
  ],
  [
    'events lane and game lane',
    [
      event(full()),
      {
        op: 'connection',
        connection: {
          session: 1,
          lane: 'events',
          account: 'Alice',
          gameId: 'AbCd1234',
          phase: 'disconnected',
        },
      },
      {
        op: 'connection',
        connection: {
          session: 1,
          lane: 'events',
          account: 'Alice',
          gameId: '',
          phase: 'auth-required',
        },
      },
      { op: 'checking' },
      {
        op: 'connection',
        connection: { session: 1, lane: 'game', account: 'Alice', gameId: '', phase: 'idle' },
      },
    ],
  ],
  [
    'a playing game refuses a new seek',
    [
      event(full()),
      start(),
      {
        op: 'connection',
        connection: {
          session: 1,
          lane: 'game',
          account: 'Alice',
          gameId: 'AbCd1234',
          phase: 'disconnected',
        },
      },
      start(),
      settle('last', true),
    ],
  ],
  [
    'finish outcomes from the local player side',
    [
      event({ type: 'gameStart', game: { gameId: 'AbCd1234', color: 'black' } }),
      { op: 'finish', winner: 'white', reason: 'resign', notify: true },
      { op: 'finish', reason: 'draw', notify: true },
      { op: 'finish', reason: 'aborted', notify: true },
      { op: 'finish', winner: 'black', reason: 'mate', notify: false },
    ],
  ],
  [
    'game start notices and game finishes by status',
    [
      event({
        type: 'gameStart',
        game: {
          gameId: 'AbCd1234',
          speed: 'correspondence',
          color: 'black',
          opponent: { username: 'Ann' },
        },
      }),
      event({ type: 'gameStart', game: { gameId: 'AbCd1234', color: 'white' } }),
      event(
        full('AbCd1234', 'd2d4', {
          white: { name: 'Stockfish', aiLevel: 6 },
          black: { id: 'alice' },
        }),
      ),
      event(live('d2d4 d7d5', { status: 'stalemate', winner: 'black' })),
      event(live('d2d4 d7d5', { status: 'resign', wdraw: true })),
    ],
  ],
  [
    'checking an interrupted connection and reset',
    [{ op: 'checking' }, { op: 'reset' }, event(full()), { op: 'reset' }, { op: 'checking' }],
  ],
  [
    'start guard during a seek',
    [start(), start(), settle('first', true, {}), settle('last', true, {})],
  ],
]

for (let seed = 1; seed <= 60; seed++)
  it(`random online game trace ${seed}`, async () => {
    const records = await play(randomSteps(seed), seed % 2 ? 'Alice' : '')
    dumps[`random/${seed}`] = records
    records.forEach((record, index) => golden.check(`random/${seed}`, index, record))
  })

SCRIPTED.forEach(([name, steps]) =>
  it(`scripted online game: ${name}`, async () => {
    const records = await play(steps, 'Alice')
    dumps[`scripted/${name}`] = records
    records.forEach((record, index) => golden.check(`scripted/${name}`, index, record))
  }),
)

afterAll(() => {
  const path = process.env.KCHESS_ONLINE_GAME_DUMP
  if (path) writeFileSync(path, JSON.stringify(dumps, null, 1))
})
