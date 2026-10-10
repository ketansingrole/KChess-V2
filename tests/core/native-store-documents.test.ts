import { createHash } from 'node:crypto'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import type { StoredReview } from '@kchess/contracts/types'
import { assertSession } from '@kchess/rules/library'
import { reviewKey } from '@kchess/rules/review'
import { nativeRules, type NativeCoreHandle } from '@kchess/native/native'
import { goldenFile } from './golden'

/**
 * The storage methods of `crates/kchess-core/src/store` (`library`, `reviewStore`, `runs`,
 * `insights`, `usage`, `voiceLog`, `setupPositionLookup`), run against a native core in a
 * temporary profile. Each step's result, and the contents of every table, is normalised and
 * checked against the golden digests recorded from the TypeScript implementation before it was
 * replaced (`golden/store-documents.json`). Seeding goes through `store.debug.exec`.
 *
 * Clock reads are normalised: a timestamp (an epoch-millisecond value in a time column or key)
 * matches any other timestamp, and each UUID is labelled by first appearance, so both sides agree
 * whenever their ids agree. Explicit `now` arguments are small numbers and compare exactly. Long
 * cells compare by digest.
 */

const golden = goldenFile('store-documents')

/** Golden checks are numbered per test, in the order the test makes them. */
const checkCounts = new Map<string, number>()
function nextCheck(): [string, number] {
  const suite = expect.getState().currentTestName ?? 'setup'
  const index = checkCounts.get(suite) ?? 0
  checkCounts.set(suite, index + 1)
  return [suite, index]
}

const NOW = 10_000
const START = 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1'

type Outcome = { value?: unknown; error?: string }

/* ── Normalisation ── */

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g
const STAMP = /"(at|updatedAt|playedAt|since|checkedAt|fetchedAt|exportedAt)":(\s*)(\d{12,})/g
const ISO_TIME = /"time":(\s*)"[^"]*"/g
const STAMP_COLUMNS = new Set(['at', 'updatedAt', 'playedAt', 'since', 'checkedAt', 'fetchedAt'])

/** The UUIDs of one side, numbered by first appearance. */
class Labels {
  private readonly indexes = new Map<string, number>()
  private readonly uuids: string[] = []

  label(uuid: string): number {
    const found = this.indexes.get(uuid)
    if (found !== undefined) return found
    const next = this.uuids.length
    this.uuids.push(uuid)
    this.indexes.set(uuid, next)
    return next
  }

  /** The label of an id this side has already shown, if any. */
  labelOf(uuid: string): number | undefined {
    return this.indexes.get(uuid)
  }

  /** The id with a label, if this side has one. */
  uuidOf(label: number): string | undefined {
    return this.uuids[label]
  }
}

function normalise(text: string, labels: Labels): string {
  return text
    .replace(STAMP, (_match, key: string, space: string) => `"${key}":${space}"<stamp>"`)
    .replace(ISO_TIME, (_match, space: string) => `"time":${space}"<iso>"`)
    .replace(UUID, (uuid) => `<u${labels.label(uuid)}>`)
}

function canonical(value: unknown, labels: Labels): string {
  const text = typeof value === 'string' ? value : (JSON.stringify(value) ?? 'null')
  return normalise(text, labels)
}

function cell(column: string, value: unknown, labels: Labels): string {
  if (typeof value === 'string') {
    const text = normalise(value, labels)
    return text.length > 200 ? `sha256:${createHash('sha256').update(text).digest('hex')}` : text
  }
  if (typeof value === 'number' && STAMP_COLUMNS.has(column) && value >= 1e12) return '<stamp>'
  return value === null ? 'NULL' : String(value)
}

const TABLES: [string, string][] = [
  ['documents', 'key'],
  ['archived_games', 'id'],
  ['games', 'account, id'],
  ['accounts', 'username'],
  ['reviews', 'key'],
  ['game_reviews', 'gameId, reviewKey'],
  ['lichess_review_checks', 'id'],
  ['runs', 'id'],
  ['usage', 'account, kind'],
  ['api_cache', 'key'],
  ['voice_log', 'id'],
  ['position_lookups', 'key'],
]

/** Rows of every table, in the order the TABLES list gives, read from the native core. */
function dumpRows(core: NativeCoreHandle, labels: Labels): Record<string, string[]> {
  const snapshot = JSON.parse(
    core.callSync('store.debug.tables', JSON.stringify([Object.fromEntries(TABLES)])),
  ) as { tables: Record<string, Record<string, unknown>[]> }
  const tables: Record<string, string[]> = {}
  for (const [table] of TABLES) {
    tables[table] = (snapshot.tables[table] ?? []).map((row) =>
      Object.entries(row)
        .map(([column, value]) => cell(column, value, labels))
        .join('\u0001'),
    )
  }
  return tables
}

/* ── Sessions ── */

interface Session {
  /** Call one method and check its result (and, unless told not to, every table). */
  run(method: string, args?: unknown[], compareTables?: boolean): Promise<Outcome>
  /** Check every table now. */
  compare(): void
  /** Insert a row, as an earlier release left it. */
  seed(sql: string, params?: unknown[]): void
  close(): Promise<void>
}

const directories: string[] = []
afterAll(() => {
  for (const directory of directories) rmSync(directory, { recursive: true, force: true })
})

function temporary(): string {
  const directory = mkdtempSync(join(tmpdir(), 'kchess-store-parity-'))
  directories.push(directory)
  return directory
}

async function attempt(work: () => unknown): Promise<Outcome> {
  try {
    return { value: await work() }
  } catch (cause) {
    return { error: cause instanceof Error ? cause.message : String(cause) }
  }
}

function describeOutcome(outcome: Outcome, labels: Labels): string {
  return outcome.error === undefined
    ? canonical(outcome.value ?? null, labels)
    : `error: ${outcome.error}`
}

/** A position-lookup entry is compared as the stored JSON it was written as; the core decodes it. */
function fromRust(method: string, value: unknown): unknown {
  if (method === 'store.setupPositionLookup.read') {
    return typeof value === 'string' ? JSON.parse(value) : null
  }
  return value
}

async function open(): Promise<Session> {
  const native = nativeRules()
  if (!native) throw new Error('The native rules are not built (pnpm run build:native).')
  const core: NativeCoreHandle = new native.NativeCore(
    { dataDir: temporary() },
    () => undefined,
    () => undefined,
  )
  const labels = new Labels()
  const call = (method: string, args: unknown[]): unknown =>
    JSON.parse(core.callSync(method, JSON.stringify(args)))
  const compare = (): void => {
    const [suite, index] = nextCheck()
    golden.check(suite, index, dumpRows(core, labels))
  }
  return {
    compare,
    async run(method, callArgs = [], compareTables = true) {
      const outcome = await attempt(() => fromRust(method, call(method, callArgs)))
      const [suite, index] = nextCheck()
      golden.check(suite, index, describeOutcome(outcome, labels))
      if (compareTables) compare()
      return outcome
    },
    seed(sql, params = []) {
      call('store.debug.exec', [sql, params])
    },
    async close() {
      await core.close()
    },
  }
}

/** The value of a call that must succeed. */
function value<T = unknown>(outcome: Outcome): T {
  if (outcome.error !== undefined) throw new Error(`unexpected error: ${outcome.error}`)
  return outcome.value as T
}

/** The message of a call that must fail. */
function failure(outcome: Outcome): string {
  if (outcome.error === undefined) throw new Error('expected an error')
  return outcome.error
}

/* ── Fixtures ── */

const game = (id: string, moves: string[] = ['e2e4'], overrides: Record<string, unknown> = {}) => ({
  id,
  source: 'board',
  startedAt: 1,
  updatedAt: 1,
  white: 'White',
  black: 'Black',
  result: '*',
  reason: 'In progress',
  finished: false,
  setup: { variant: 'standard', fen: START },
  moves,
  timeControl: '-',
  ...overrides,
})

const study = (name: string, pgn = '1. e4 *') => ({ op: 'save', name, pgn })

const chapter = (name: string, pgn = '1. e4 *') => ({ name, pgn })

const review = (overrides: Partial<StoredReview> = {}): StoredReview => ({
  key: reviewKey(START, ['e2e4', 'e7e5']),
  fen: START,
  moves: ['e2e4', 'e7e5'],
  source: 'local',
  complete: true,
  depth: 18,
  updatedAt: 1,
  evals: [{ cp: 0 }, { cp: 0 }, { cp: 0 }],
  ...overrides,
})

/** A review in which the analysis finds a mistake to drill, found by a seeded search. */
function mistakeReview(): StoredReview {
  const moves = ['e2e4', 'e7e5', 'g1f3', 'b8c6']
  const candidates = [
    ['d2d4', 'c2c4', 'b1c3'],
    ['c7c5', 'g8f6', 'd7d5'],
    ['f1c4', 'd2d4', 'b1c3'],
    ['f8c5', 'g8f6', 'a7a6'],
    [],
  ]
  const random = generator(7)
  for (let trial = 0; trial < 400; trial++) {
    const evals = candidates.map((options, index) => {
      const cp = Math.round((random() - 0.5) * 1200)
      const best = options[Math.floor(random() * options.length)]
      if (best === undefined || index === moves.length) return { cp, depth: 18 }
      return { cp, best, depth: 18, pv: random() < 0.5 ? [best, 'e7e5'] : undefined }
    })
    const candidate = review({ moves, evals })
    // The review analysis the core makes of the scores (engine lines do not count).
    const analysis = JSON.parse(
      nativeRules()!.analyseReview(
        JSON.stringify({
          key: candidate.key,
          source: candidate.source,
          complete: candidate.complete,
          fen: candidate.fen,
          moves: candidate.moves,
          evals: candidate.evals.map((e) =>
            e && typeof e === 'object' ? { cp: e.cp, mate: e.mate } : e,
          ),
          judgments: candidate.judgments,
          accuracy: candidate.accuracy,
        }),
      ) as string,
    ) as { moves: { judgment?: string }[] }
    const drillable = analysis.moves.some(
      (move, index) =>
        Boolean(move.judgment) &&
        evals[index]?.best !== undefined &&
        evals[index]?.best !== moves[index],
    )
    if (drillable) return candidate
  }
  throw new Error('No review with a drillable mistake was found.')
}

/** A seeded generator, so seeded data is the same on every run. */
function generator(seed: number): () => number {
  let state = seed
  return () => {
    state = (state * 1103515245 + 12345) & 0x7fffffff
    return state / 0x80000000
  }
}

function seedGame(s: Session, id: string, account = 'Alice', extra: Record<string, unknown> = {}) {
  const values: Record<string, unknown> = {
    account,
    id,
    createdAt: 1,
    lastMoveAt: 1,
    rated: 0,
    speed: 'blitz',
    perf: 'blitz',
    status: 'resign',
    winner: null,
    color: 'white',
    opponent: 'Bob',
    opponentRating: null,
    playerRating: null,
    ratingDiff: null,
    opening: null,
    moves: 'e4 e5',
    pgn: null,
    ...extra,
  }
  const columns = Object.keys(values)
  s.seed(
    `INSERT INTO games (${columns.join(', ')}) VALUES (${columns.map(() => '?').join(', ')})`,
    Object.values(values),
  )
}

/* ── Library documents ── */

describe('library documents', () => {
  it('reads the library, takes earlier documents over once and reads damaged ones as empty', async () => {
    const s = await open()
    try {
      value(await s.run('store.library.library', [NOW]))
      const legacy = {
        'kchess:studies:v1': JSON.stringify({
          version: 1,
          items: [{ id: 'old', name: 'Old study', pgn: '1. e4 *', updatedAt: 1 }],
        }),
        'kchess:game-history:v1': JSON.stringify({ version: 1, games: [game('b'), game('a')] }),
        'kchess:computer:v1': JSON.stringify({ version: 1, moves: ['e2e5'], level: 'club' }),
        'kchess:analysis:v1': '{not json',
        'kchess:history-session:board': JSON.stringify({ id: 'board-1', startedAt: 5 }),
        'kchess:local:v1': JSON.stringify(
          assertSession('local', {
            setup: { variant: 'standard', fen: START },
            moves: ['e2e4'],
            clock: null,
            times: null,
            result: null,
          }),
        ),
        'kchess:mistakes:v1': JSON.stringify({
          version: 1,
          items: [
            {
              id: 'm1',
              fen: START,
              solution: ['e2e4'],
              judgment: 'blunder',
              dueAt: 1,
              streak: 0,
              attempts: 0,
            },
            {
              id: 'bad',
              fen: START,
              solution: ['e2e5'],
              judgment: 'x',
              dueAt: 1,
              streak: 0,
              attempts: 0,
            },
          ],
        }),
        'kchess:tournaments-joined': JSON.stringify([
          { system: 'arena', id: 'live', account: 'Alice', name: 'Live', until: NOW + 5 },
          { system: 'arena', id: 'ended', account: 'Alice', name: 'Ended', until: NOW - 5 },
        ]),
        'kchess:repertoire-misses': JSON.stringify({ 'study:white': { [START]: 2 } }),
      }
      const snapshot = value<{ imported: boolean; games: { id: string }[] }>(
        await s.run('store.library.importLibrary', [legacy]),
      )
      expect(snapshot.imported).toBe(true)
      // The most recent game stays most recent.
      expect(snapshot.games.map((g) => g.id)).toEqual(['b', 'a'])
      value(
        await s.run('store.library.importLibrary', [
          { 'kchess:studies:v1': '{"version":2,"items":[]}' },
        ]),
      )
      value(await s.run('store.library.library', [NOW]))
      value(await s.run('store.library.joinedTournaments', [NOW]))

      // A damaged study document reads as no studies; the next command writes over it.
      s.seed("UPDATE documents SET body = '{broken' WHERE key = 'studies'")
      value(await s.run('store.library.library', [NOW]))
      value(await s.run('store.library.studyCommand', [study('After damage'), NOW]))
      // Documents of the wrong shape read as empty, and an archived game that does not replay is skipped.
      s.seed('UPDATE documents SET body = \'{"version":2,"items":"nope"}\' WHERE key = \'studies\'')
      s.seed("UPDATE documents SET body = '[1]' WHERE key = 'repertoire:misses'")
      s.seed("UPDATE documents SET body = '{}' WHERE key = 'tournaments:joined'")
      s.seed(
        'INSERT INTO archived_games (id, body, seq) VALUES (\'broken\', \'{"id":"broken"}\', 99999)',
      )
      value(await s.run('store.library.library', [NOW]))
    } finally {
      await s.close()
    }
  })

  it('skips a saved document over the size limit, and the import still completes', async () => {
    const s = await open()
    try {
      const huge = JSON.stringify({ version: 2, items: [], pad: 'x'.repeat(2_000_001) })
      expect(
        value<{ imported: boolean; studies: unknown[] }>(
          await s.run('store.library.importLibrary', [{ 'kchess:studies:v1': huge }]),
        ),
      ).toMatchObject({ imported: true, studies: [] })
    } finally {
      await s.close()
    }
  })

  it('skips an archive over the game limit as a whole and imports the other documents', async () => {
    const s = await open()
    try {
      const games = Array.from({ length: 501 }, (_, n) => game(`g${n}`))
      value(
        await s.run('store.library.importLibrary', [
          {
            'kchess:studies:v1': JSON.stringify({
              version: 1,
              items: [{ id: 'x', name: 'X', pgn: '1. e4 *', updatedAt: 1 }],
            }),
            'kchess:game-history:v1': JSON.stringify({ version: 1, games }),
          },
        ]),
      )
      const after = value<{ studies: unknown[]; games: unknown[] }>(
        await s.run('store.library.library', [NOW]),
      )
      expect(after.studies).toHaveLength(1)
      expect(after.games).toEqual([])
    } finally {
      await s.close()
    }
  })

  it('skips an earlier session document that is not valid for its kind', async () => {
    const s = await open()
    try {
      value(
        await s.run('store.library.importLibrary', [
          {
            'kchess:history-session:board': JSON.stringify({ id: 'board-2', startedAt: 9 }),
            'kchess:analysis:v1': JSON.stringify({
              pgn: '1. e5 *',
              path: '',
              orientation: 'white',
              study: '',
              chapter: '',
            }),
            'kchess:history-session:clock': 'null',
          },
        ]),
      )
      value(await s.run('store.library.library', [NOW]))
    } finally {
      await s.close()
    }
  })
})

/* ── Studies ── */

describe('studies', () => {
  it('creates, edits, copies, restores and removes studies, refusing invalid commands', async () => {
    const s = await open()
    try {
      const created = value<{ id: string; studies: { chapters: { id: string }[] }[] }>(
        await s.run('store.library.studyCommand', [
          {
            op: 'saveChapters',
            name: '  Repertoire  ',
            chapters: [chapter('One'), chapter('Two', '1. d4 *')],
          },
          NOW,
        ]),
      )
      const id = created.id
      const first = created.studies[0]!.chapters[0]!.id
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'saveChapters', name: 'Broken', chapters: [chapter('x', '1. e5 *')] },
            NOW,
          ]),
        ),
      ).toContain('valid chapters')
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'saveChapters', name: '', chapters: [chapter('x')] },
            NOW,
          ]),
        ),
      ).toContain('valid chapters')
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'saveChapters', name: 'Empty', chapters: [] },
            NOW,
          ]),
        ),
      ).toContain('valid chapters')
      expect(failure(await s.run('store.library.studyCommand', [study('   '), NOW]))).toBe(
        'Name the study.',
      )
      // Saving a study with an id replaces its first chapter; a chapter id picks another.
      value(
        await s.run('store.library.studyCommand', [
          { op: 'save', id, name: 'Renamed', pgn: '1. c4 *' },
          NOW,
        ]),
      )
      value(
        await s.run('store.library.studyCommand', [
          { op: 'save', id, name: 'Renamed', chapterId: first, pgn: '1. b3 *' },
          NOW,
        ]),
      )
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'save', id, name: 'Renamed', chapterId: 'nope', pgn: '1. b3 *' },
            NOW,
          ]),
        ),
      ).toBe('That chapter cannot be saved.')
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'save', id, name: 'Renamed', chapterId: first, pgn: '1. e5 *' },
            NOW,
          ]),
        ),
      ).toBe('That chapter cannot be saved.')
      // Without a name, the TypeScript code fails on `name.trim()`; the Rust core fails the same way.
      expect(
        failure(
          await s.run('store.library.studyCommand', [{ op: 'save', id, pgn: '1. b3 *' }, NOW]),
        ),
      ).toBe("Cannot read properties of undefined (reading 'trim')")
      value(
        await s.run('store.library.studyCommand', [{ op: 'addChapter', id, name: ' Three ' }, NOW]),
      )
      expect(
        failure(
          await s.run('store.library.studyCommand', [{ op: 'addChapter', id, name: '  ' }, NOW]),
        ),
      ).toBe('Name the new chapter.')
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'addChapter', id: 'missing', name: 'x' },
            NOW,
          ]),
        ),
      ).toBe('Name the new chapter.')
      const chapters = value<{ studies: { id: string; chapters: { id: string }[] }[] }>(
        await s.run('store.library.library', [NOW]),
      ).studies.find((entry) => entry.id === id)!.chapters
      value(
        await s.run('store.library.studyCommand', [
          { op: 'renameChapter', id, chapterId: chapters[1]!.id, name: 'Second line' },
          NOW,
        ]),
      )
      value(
        await s.run('store.library.studyCommand', [
          { op: 'renameChapter', id, chapterId: chapters[1]!.id, name: '   ' },
          NOW,
        ]),
      )
      const copy = value<{ id: string }>(
        await s.run('store.library.studyCommand', [
          { op: 'duplicateChapter', id, chapterId: chapters[0]!.id },
          NOW,
        ]),
      )
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'duplicateChapter', id, chapterId: 'gone' },
            NOW,
          ]),
        ),
      ).toBe('That chapter no longer exists.')
      value(
        await s.run('store.library.studyCommand', [
          { op: 'removeChapter', id, chapterId: copy.id },
          NOW,
        ]),
      )
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'removeChapter', id, chapterId: 'gone' },
            NOW,
          ]),
        ),
      ).toBe('That chapter no longer exists.')
      const whole = value<{ id: string }>(
        await s.run('store.library.studyCommand', [{ op: 'duplicate', id }, NOW]),
      )
      value(
        await s.run('store.library.studyCommand', [
          { op: 'rename', id, name: ' Final name ' },
          NOW,
        ]),
      )
      value(await s.run('store.library.studyCommand', [{ op: 'rename', id, name: '   ' }, NOW]))
      value(await s.run('store.library.studyCommand', [{ op: 'remove', id }, NOW]))
      value(await s.run('store.library.studyCommand', [{ op: 'remove', id: 'gone' }, NOW]))
      value(
        await s.run('store.library.studyCommand', [
          {
            op: 'restore',
            study: {
              id,
              name: 'Back',
              pgn: '1. e4 *',
              chapters: [{ id: 'c1', name: 'Only', pgn: '1. e4 *' }],
              updatedAt: NOW,
            },
          },
          NOW,
        ]),
      )
      // Restoring a study that is present changes nothing.
      value(
        await s.run('store.library.studyCommand', [
          {
            op: 'restore',
            study: {
              id,
              name: 'Ignored',
              pgn: '1. e4 *',
              chapters: [{ id: 'c2', name: 'Only', pgn: '1. e4 *' }],
              updatedAt: NOW,
            },
          },
          NOW,
        ]),
      )
      value(await s.run('store.library.studyCommand', [{ op: 'remove', id: whole.id }, NOW]))
      value(await s.run('store.library.studyCommand', [{ op: 'unknown' }, NOW]))
    } finally {
      await s.close()
    }
  })

  it('keeps the library unchanged when a command fails', async () => {
    const s = await open()
    try {
      const created = value<{ id: string; studies: { chapters: { id: string }[] }[] }>(
        await s.run('store.library.studyCommand', [
          { op: 'saveChapters', name: 'Repertoire', chapters: [chapter('One')] },
          NOW,
        ]),
      )
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'removeChapter', id: created.id, chapterId: created.studies[0]!.chapters[0]!.id },
            NOW,
          ]),
        ),
      ).toContain('at least one chapter')
    } finally {
      await s.close()
    }
  })

  it('refuses a 51st study and a study too large for the library', async () => {
    const s = await open()
    try {
      for (let n = 0; n < 50; n++)
        value(await s.run('store.library.studyCommand', [study(`Study ${n}`), NOW], false))
      s.compare()
      expect(
        failure(await s.run('store.library.studyCommand', [study('One more'), NOW])),
      ).toContain('full')
      expect(
        value<{ studies: unknown[] }>(await s.run('store.library.library', [NOW])).studies,
      ).toHaveLength(50)
      const long = `1. e4 {${'x'.repeat(60_000)}} *`
      const chapters = Array.from({ length: 64 }, (_, n) => chapter(`Chapter ${n}`, long))
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'saveChapters', name: 'Huge', chapters },
            NOW,
          ]),
        ),
      ).toContain('The study library is full')
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            { op: 'saveChapters', name: 'Too many', chapters: [...chapters, chapter('x')] },
            NOW,
          ]),
        ),
      ).toContain('valid chapters')
    } finally {
      await s.close()
    }
  })

  it('links a study to its cloud copy, keeps a changed local copy and refuses an unreadable download', async () => {
    const s = await open()
    try {
      const saved = value<{ id: string }>(
        await s.run('store.library.studyCommand', [
          { op: 'saveChapters', name: 'Online', chapters: [chapter('Main', '1. e4 e5 *')] },
          NOW,
        ]),
      )
      value(
        await s.run('store.library.studyCommand', [
          {
            op: 'markCloud',
            id: saved.id,
            account: 'Alice',
            remoteId: 'abcd1234',
            baseline: '1. e4 e5 *',
          },
          NOW,
        ]),
      )
      // The copy still matches what was downloaded, so the study is updated in place.
      const same = value<{ id: string; conflict: boolean }>(
        await s.run('store.library.studyCommand', [
          {
            op: 'offline',
            account: 'alice',
            remote: { id: 'abcd1234', name: 'Online' },
            chapters: [chapter('Main', '1. d4 d5 *')],
          },
          NOW,
        ]),
      )
      expect(same.id).toBe(saved.id)
      expect(same.conflict).toBe(false)
      // A local edit since the download is kept; the cloud version becomes a separate copy.
      value(
        await s.run('store.library.studyCommand', [
          { op: 'save', id: same.id, name: 'Online', pgn: '1. c4 c5 *' },
          NOW,
        ]),
      )
      const copy = value<{ id: string; conflict: boolean }>(
        await s.run('store.library.studyCommand', [
          {
            op: 'offline',
            account: 'Alice',
            remote: { id: 'abcd1234', name: 'Online' },
            chapters: [chapter('Main', '1. g3 *')],
          },
          NOW,
        ]),
      )
      expect(copy.conflict).toBe(true)
      expect(copy.id).not.toBe(same.id)
      // A downloaded chapter with no game is refused.
      expect(
        failure(
          await s.run('store.library.studyCommand', [
            {
              op: 'offline',
              account: 'Alice',
              remote: { id: 'abcd1234', name: 'Online' },
              chapters: [chapter('Bad', '1. e5 *')],
            },
            NOW,
          ]),
        ),
      ).toContain('valid chapters')
    } finally {
      await s.close()
    }
  })
})

/* ── Played games and sessions ── */

describe('played games', () => {
  it('lists the most recently saved game first, replaces games in place and removes them', async () => {
    const s = await open()
    try {
      value(await s.run('store.library.saveArchivedGame', [game('a')]))
      value(await s.run('store.library.saveArchivedGame', [game('b')]))
      value(await s.run('store.library.saveArchivedGame', [game('a', ['e2e4', 'e7e5'])]))
      expect(
        value<{ games: { id: string }[] }>(await s.run('store.library.library', [NOW])).games.map(
          (g) => g.id,
        ),
      ).toEqual(['a', 'b'])
      value(await s.run('store.library.removeArchivedGame', ['a']))
      value(await s.run('store.library.removeArchivedGame', ['missing']))
      expect(
        value<{ games: { id: string }[] }>(await s.run('store.library.library', [NOW])).games.map(
          (g) => g.id,
        ),
      ).toEqual(['b'])
    } finally {
      await s.close()
    }
  })

  it('stores a game as sent, skips one that does not replay when read, and never drops older games', async () => {
    const s = await open()
    try {
      value(await s.run('store.library.saveArchivedGame', [game('illegal', ['e2e4', 'e2e4'])]))
      expect(
        value<{ games: unknown[] }>(await s.run('store.library.library', [NOW])).games,
      ).toEqual([])
      for (let n = 0; n < 499; n++)
        value(await s.run('store.library.saveArchivedGame', [game(String(n))], false))
      s.compare()
      expect(failure(await s.run('store.library.saveArchivedGame', [game('overflow')]))).toContain(
        'Game history is full',
      )
      value(
        await s.run('store.library.saveArchivedGame', [
          { ...game('0'), result: '1-0', finished: true },
        ]),
      )
      // The illegal game counts toward the limit but is not listed.
      expect(
        value<{ games: unknown[] }>(await s.run('store.library.library', [NOW])).games,
      ).toHaveLength(499)
    } finally {
      await s.close()
    }
  }, 60_000)

  it('stops at the size limit before the count limit with long legal games', async () => {
    const s = await open()
    try {
      const shuffle = ['g1f3', 'g8f6', 'f3g1', 'f6g8']
      const moves = Array.from({ length: 1024 }, (_, n) => shuffle[n % 4]!)
      let saved = 0
      for (;;) {
        const outcome = await s.run(
          'store.library.saveArchivedGame',
          [game(String(saved), moves)],
          false,
        )
        if (outcome.error !== undefined) {
          expect(outcome.error).toContain('Game history is full')
          break
        }
        saved++
      }
      expect(saved).toBeLessThan(500)
      s.compare()
    } finally {
      await s.close()
    }
  })
})

describe('sessions', () => {
  it('stores a session with its version, reads it back, and reads damaged or illegal ones as none', async () => {
    const s = await open()
    try {
      const local = assertSession('local', {
        setup: { variant: 'standard', fen: START },
        moves: ['e2e4'],
        clock: null,
        times: null,
        result: null,
      })
      value(await s.run('store.library.saveSession', ['local', local]))
      expect(value(await s.run('store.library.readSession', ['local']))).toEqual(local)
      // A session is saved as sent (only its decoding checks it), so an illegal one reads back as none.
      value(await s.run('store.library.saveSession', ['local', { ...local, moves: ['e2e5'] }]))
      expect(value(await s.run('store.library.readSession', ['local']))).toBeFalsy()
      expect(value(await s.run('store.library.readSession', ['computer']))).toBeFalsy()
      value(
        await s.run('store.library.saveSession', [
          'analysis',
          { pgn: '1. e4 *', path: '', orientation: 'white', study: '', chapter: '' },
        ]),
      )
      expect(value(await s.run('store.library.readSession', ['analysis']))).toMatchObject({
        pgn: '1. e4 *',
      })
      s.seed("UPDATE documents SET body = '{{' WHERE key = 'session:computer'")
      expect(value(await s.run('store.library.readSession', ['computer']))).toBeFalsy()
      expect(
        failure(
          await s.run('store.library.saveSession', ['analysis', { pgn: 'x'.repeat(2_100_000) }]),
        ),
      ).toBe('This session is too large to save automatically. Export a PGN copy.')
    } finally {
      await s.close()
    }
  })
})

describe('tournaments and repertoire notes', () => {
  it('forgets ended tournaments and those of signed-out accounts, and skips malformed entries', async () => {
    const s = await open()
    try {
      value(
        await s.run('store.library.rememberTournament', [
          { system: 'arena', id: 'ended', account: 'Alice', name: 'Old', until: NOW - 1 },
          NOW,
        ]),
      )
      value(
        await s.run('store.library.rememberTournament', [
          { system: 'arena', id: 'live', account: 'Alice', name: 'Live', until: NOW + 60_000 },
          NOW,
        ]),
      )
      value(
        await s.run('store.library.rememberTournament', [
          { system: 'swiss', id: 'other', account: 'Bob', name: 'Swiss', until: NOW + 60_000 },
          NOW,
        ]),
      )
      // The same tournament again replaces the earlier entry.
      value(
        await s.run('store.library.rememberTournament', [
          { system: 'swiss', id: 'other', account: 'Bob', name: 'Swiss 2', until: NOW + 90_000 },
          NOW,
        ]),
      )
      expect(
        value<{ id: string }[]>(await s.run('store.library.joinedTournaments', [NOW])).map(
          (t) => t.id,
        ),
      ).toEqual(['live', 'other'])
      value(await s.run('store.library.forgetTournament', ['swiss', 'other']))
      value(await s.run('store.library.forgetTournamentsOf', [['alice']]))
      value(await s.run('store.library.forgetTournamentsOf', [[]]))
      // A malformed entry is stored as sent and dropped when read.
      value(await s.run('store.library.rememberTournament', [{ system: 'arena', id: 'bad' }, NOW]))
      value(await s.run('store.library.joinedTournaments', [NOW]))
    } finally {
      await s.close()
    }
  })

  it('counts missed repertoire moves per study and side, and recovers from a damaged file', async () => {
    const s = await open()
    try {
      const fen = 'rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1'
      expect(
        value(await s.run('store.library.recordRepertoireMiss', ['study:white', fen])),
      ).toEqual({
        'study:white': { [fen]: 1 },
      })
      expect(
        value(await s.run('store.library.recordRepertoireMiss', ['study:white', fen])),
      ).toEqual({
        'study:white': { [fen]: 2 },
      })
      expect(value(await s.run('store.library.clearRepertoireMisses', ['study:white']))).toEqual({})
      expect(value(await s.run('store.library.clearRepertoireMisses', ['study:none']))).toEqual({})
      s.seed("UPDATE documents SET body = 'nope' WHERE key = 'repertoire:misses'")
      value(await s.run('store.library.recordRepertoireMiss', ['study:black', fen]))
    } finally {
      await s.close()
    }
  })
})

/* ── Mistake drills ── */

describe('mistake drills', () => {
  it('turns a completed review into drills, answers them, and adds nothing twice', async () => {
    const s = await open()
    try {
      const drilled = mistakeReview()
      value(await s.run('store.reviewStore.writeReview', [drilled]))
      const added = value<{ added: number; items: { id: string }[] }>(
        await s.run('store.library.addMistakes', [drilled.key, undefined, NOW]),
      )
      expect(added.added).toBeGreaterThan(0)
      expect(
        value<{ added: number }>(
          await s.run('store.library.addMistakes', [drilled.key, undefined, NOW]),
        ).added,
      ).toBe(0)
      value(await s.run('store.library.addMistakes', [drilled.key, 'black', NOW]))
      value(await s.run('store.library.addMistakes', [drilled.key, 'white', NOW + 1]))
      expect(
        value<{ added: number }>(
          await s.run('store.library.addMistakes', ['missing', undefined, NOW]),
        ).added,
      ).toBe(0)
      value(await s.run('store.library.answerMistake', [added.items[0]!.id, true, NOW]))
      value(await s.run('store.library.answerMistake', [added.items[0]!.id, false, NOW]))
      value(await s.run('store.library.answerMistake', ['unknown', true, NOW]))
    } finally {
      await s.close()
    }
  })

  it('reads at most 500 drills and keeps the newest 500 when adding', async () => {
    const s = await open()
    try {
      const items = Array.from({ length: 600 }, (_, n) => ({
        id: `m${n}`,
        fen: START,
        solution: ['e2e4'],
        judgment: 'blunder',
        dueAt: 1,
        streak: 0,
        attempts: 0,
      }))
      s.seed("INSERT INTO documents (key, body, updatedAt) VALUES ('mistakes', ?, 1)", [
        JSON.stringify({ version: 1, items }),
      ])
      expect(
        value<{ mistakes: unknown[] }>(await s.run('store.library.library', [NOW])).mistakes,
      ).toHaveLength(500)
      // Beyond the read limit, a drill is not there to answer.
      value(await s.run('store.library.answerMistake', ['m599', true, NOW]))
      value(await s.run('store.library.answerMistake', ['m10', true, NOW]))
    } finally {
      await s.close()
    }
  })
})

/* ── Reviews ── */

describe('game reviews', () => {
  it('retains every association through updates and excludes each account copy from the queue', async () => {
    const s = await open()
    try {
      seedGame(s, 'Game0001')
      seedGame(s, 'Game0002')
      seedGame(s, 'Game0001', 'Bob')
      value(await s.run('store.reviewStore.writeReview', [{ ...review(), gameId: 'Game0001' }]))
      value(await s.run('store.reviewStore.writeReview', [{ ...review(), gameId: 'Game0002' }]))
      value(
        await s.run('store.reviewStore.writeReview', [
          { ...review(), gameId: 'Game0002', depth: 20 },
        ]),
      )
      expect(
        Object.keys(
          value<object>(
            await s.run('store.reviewStore.reviewSummaries', [['Game0001', 'Game0002']]),
          ),
        ),
      ).toEqual(['Game0001', 'Game0002'])
      expect(value(await s.run('store.reviewStore.gamesToReview', [['Alice', 'Bob']]))).toEqual([])
      expect(value(await s.run('store.insights.insights', [{ account: 'Alice' }]))).toMatchObject({
        accuracy: { games: 2 },
      })
    } finally {
      await s.close()
    }
  })

  it('links a second game even when a stronger review prevents replacing its evaluations', async () => {
    const s = await open()
    try {
      value(
        await s.run('store.reviewStore.writeReview', [
          { ...review(), source: 'lichess', gameId: 'Game0001' },
        ]),
      )
      const result = value<{ review: { source: string; complete: boolean; gameId: string } }>(
        await s.run('store.reviewStore.writeReview', [
          { ...review({ complete: false }), gameId: 'Game0002' },
        ]),
      )
      expect(result.review).toMatchObject({ source: 'lichess', complete: true, gameId: 'Game0002' })
      expect(
        value<{ source: string }>(await s.run('store.reviewStore.readReview', [review().key]))
          .source,
      ).toBe('lichess')
      expect(
        value<Record<string, unknown>>(
          await s.run('store.reviewStore.reviewSummaries', [['Game0001', 'Game0002']]),
        ),
      ).toHaveProperty('Game0002')
    } finally {
      await s.close()
    }
  })

  it('retains both associations when identical searches are stored before their first output', async () => {
    const s = await open()
    try {
      value(await s.run('store.reviewStore.writeReview', [{ ...review(), gameId: 'Game0002' }]))
      value(
        await s.run('store.reviewStore.writeReview', [
          { ...review({ complete: false }), gameId: 'Game0001' },
        ]),
      )
      expect(
        Object.keys(
          value<object>(
            await s.run('store.reviewStore.reviewSummaries', [['Game0001', 'Game0002']]),
          ),
        ),
      ).toHaveLength(2)
    } finally {
      await s.close()
    }
  })

  it('reads and merges reviews, and refuses one that cannot be analysed', async () => {
    const s = await open()
    try {
      expect(value(await s.run('store.reviewStore.readReview', ['missing']))).toBeNull()
      const first = value<{ summary: { white: object } }>(
        await s.run('store.reviewStore.writeReview', [review({ gameId: 'Game0009' })]),
      )
      expect(first.summary.white).toBeDefined()
      // A finished local review beats an unfinished one and keeps the game it was first saved for.
      value(await s.run('store.reviewStore.writeReview', [review({ complete: false, depth: 4 })]))
      expect(
        value<{ depth: number; gameId: string }>(
          await s.run('store.reviewStore.readReview', [review().key]),
        ),
      ).toMatchObject({ depth: 18, gameId: 'Game0009' })
      value(await s.run('store.reviewStore.writeReview', [review({ gameId: 'Game0010' })]))
      // No evaluations at all cannot be read.
      expect(
        failure(
          await s.run('store.reviewStore.writeReview', [
            review({ evals: undefined as never, key: 'no-evals' }),
          ]),
        ),
      ).toBe("Cannot read properties of undefined (reading 'map')")
      // A review whose evaluations do not fit its moves is analysed the same way on both sides.
      void (await s.run('store.reviewStore.writeReview', [
        review({ moves: ['e2e4'], key: 'short' }),
      ]))
    } finally {
      await s.close()
    }
  })
})

describe('game review lists and accounts', () => {
  it('lists games to review, marks checked games, tells accounts apart by case and counts reviews', async () => {
    const s = await open()
    try {
      s.seed("INSERT INTO accounts (username, connected, lastSyncedAt) VALUES ('Alice', 1, NULL)")
      seedGame(s, 'GameAAAA', 'Alice', {
        createdAt: 5_000,
        pgn: '1. e4 e5 *',
        opening: 'Ruy Lopez',
      })
      seedGame(s, 'GameBBBB', 'alice', { createdAt: 9_000, moves: '' })
      seedGame(s, 'GameCCCC', 'Alice', { createdAt: 7_000, status: 'created' })
      seedGame(s, 'GameDDDD', 'Bob', { createdAt: 8_000 })
      seedGame(s, 'GameEEEE', 'Alice', { createdAt: 2_000 })
      value(
        await s.run('store.reviewStore.writeReview', [
          { ...review(), gameId: 'GameEEEE', key: 'done' },
        ]),
      )
      value(
        await s.run('store.reviewStore.writeReview', [
          { ...review({ complete: false }), gameId: 'GameAAAA', key: 'partial' },
        ]),
      )
      expect(value(await s.run('store.reviewStore.hasAccount', ['alice']))).toBe(true)
      expect(value(await s.run('store.reviewStore.hasAccount', ['carol']))).toBe(false)
      expect(value(await s.run('store.reviewStore.gamesToReview', [['Alice']]))).toHaveLength(1)
      value(await s.run('store.reviewStore.markChecked', [['GameAAAA'], NOW]))
      value(await s.run('store.reviewStore.markChecked', [[], NOW]))
      expect(
        value(await s.run('store.reviewStore.gamesToReview', [['Alice'], 4_000, 1])),
      ).toHaveLength(1)
      expect(value(await s.run('store.reviewStore.gamesToReview', [[]]))).toEqual([])
      value(await s.run('store.reviewStore.markChecked', [['GameBBBB', 'GameCCCC']]))
      expect(value(await s.run('store.reviewStore.reviewCount', []))).toBe(2)
      expect(value(await s.run('store.reviewStore.reviewSummaries', [[]]))).toEqual({})
      expect(value(await s.run('store.reviewStore.reviewSummaries', [['missing']]))).toEqual({})
    } finally {
      await s.close()
    }
  })
})

/* ── Runs ── */

describe('runs', () => {
  it('records runs, reports the best of each variant and the newest twenty, and clears them', async () => {
    const s = await open()
    try {
      expect(value(await s.run('store.runs.runSummary', ['storm']))).toEqual({
        kind: 'storm',
        best: {},
        recent: [],
        total: 0,
      })
      value(
        await s.run('store.runs.saveRun', [
          { kind: 'storm', variant: '', score: 12, detail: { moves: 12 } },
        ]),
      )
      expect(
        value<{ isBest: boolean }>(
          await s.run('store.runs.saveRun', [
            { kind: 'storm', variant: '', score: 12, detail: {} },
          ]),
        ),
      ).toMatchObject({ isBest: false })
      expect(
        value<{ isBest: boolean }>(
          await s.run('store.runs.saveRun', [{ kind: 'storm', variant: '', score: 0, detail: {} }]),
        ),
      ).toMatchObject({ isBest: false })
      expect(
        value<{ isBest: boolean }>(
          await s.run('store.runs.saveRun', [
            { kind: 'storm', variant: '', score: 15.5, detail: { nested: [1, { x: null }] } },
          ]),
        ),
      ).toMatchObject({ isBest: true })
      expect(
        value<{ isBest: boolean }>(
          await s.run('store.runs.saveRun', [
            { kind: 'rush', variant: 'blitz', score: 3, detail: {} },
          ]),
        ),
      ).toMatchObject({ isBest: true })
      for (let n = 0; n < 25; n++)
        value(
          await s.run(
            'store.runs.saveRun',
            [{ kind: 'streak', variant: 'a', score: n, detail: {} }],
            false,
          ),
        )
      s.compare()
      const summary = value<{
        recent: unknown[]
        total: number
        best: Record<string, { score: number }>
      }>(await s.run('store.runs.runSummary', ['streak']))
      expect(summary.recent).toHaveLength(20)
      expect(summary.total).toBe(25)
      expect(summary.best.a?.score).toBe(24)
      s.seed("UPDATE runs SET detail = '{nope' WHERE kind = 'rush'")
      value(await s.run('store.runs.runSummary', ['rush']))
      value(await s.run('store.runs.clearRuns', ['rush']))
      value(await s.run('store.runs.clearRuns', [null]))
      expect(value<{ total: number }>(await s.run('store.runs.runSummary', ['storm'])).total).toBe(
        0,
      )
    } finally {
      await s.close()
    }
  })
})

/* ── Insights ── */

describe('insights', () => {
  it('reports results, openings, ratings, lengths, streaks and accuracy the same with and without filters', async () => {
    const s = await open()
    try {
      const random = generator(42)
      const pick = <T>(items: readonly T[]): T => items[Math.floor(random() * items.length)]!
      // A fixed clock for the fixtures: rows stay byte-identical from run to run (the golden
      // digests compare them), and every row is newer than any `days` window the queries use.
      const now = 4_000_000_000_000
      const openings = [
        'Sicilian Defense: Najdorf',
        'Sicilian Defense',
        'Italian Game: Giuoco',
        '  Queen’s Gambit ',
        ':odd',
        null,
      ]
      for (let n = 0; n < 120; n++) {
        const hours = Math.floor(random() * 24 * 200)
        const createdAt = now - hours * 3_600_000 - 1_234
        const plies = pick([0, 19, 20, 39, 40, 79, 80, 119, 120, 150])
        const moves = plies === 0 ? '' : Array.from({ length: plies }, () => 'e4').join(' ')
        seedGame(s, `Ins${String(n).padStart(4, '0')}`, n % 3 === 0 ? 'Alice' : 'alice', {
          createdAt,
          lastMoveAt: createdAt,
          rated: pick([0, 1]),
          speed: pick(['bullet', 'blitz', 'rapid', 'classical']),
          status: pick(['mate', 'resign', 'outoftime', 'draw', 'created', 'started']),
          winner: pick(['white', 'black', null]),
          color: pick(['white', 'black']),
          opponentRating: pick([1200, 1300, 1400, 1500, 1600, null]),
          playerRating: pick([1200, 1250, 1300, 1400, 1450]),
          opening: pick(openings),
          moves,
        })
      }
      // Rating differences on the band edges, and an opening family seen twice.
      seedGame(s, 'EdgeA', 'Alice', {
        createdAt: now - 1000,
        opponentRating: 1500,
        playerRating: 1300,
        winner: 'white',
        color: 'white',
        status: 'mate',
        moves: 'a b',
      })
      seedGame(s, 'EdgeB', 'Alice', {
        createdAt: now - 2000,
        opponentRating: 1250,
        playerRating: 1300,
        winner: 'black',
        color: 'white',
        status: 'mate',
        moves: 'a b',
      })
      seedGame(s, 'EdgeC', 'Alice', {
        createdAt: now - 3000,
        opponentRating: 1349,
        playerRating: 1300,
        winner: null,
        color: 'black',
        status: 'draw',
        moves: 'a b',
      })
      seedGame(s, 'EdgeD', 'Alice', {
        createdAt: now - 4000,
        opponentRating: 1700,
        playerRating: 1300,
        winner: 'black',
        color: 'white',
        status: 'resign',
        moves: 'a b',
        opening: 'Sicilian Defense: Taimanov',
      })
      // Some games have a finished review, one of them with only one side summarised.
      value(
        await s.run('store.reviewStore.writeReview', [
          { ...review(), key: 'r1', gameId: 'Ins0000' },
        ]),
      )
      value(
        await s.run('store.reviewStore.writeReview', [
          { ...review(), key: 'r2', gameId: 'Ins0003' },
        ]),
      )
      const queries = [
        { account: 'Alice' },
        { account: 'alice', speed: 'blitz' },
        { account: 'alice', rated: true },
        { account: 'alice', rated: false, days: 30 },
        { account: 'alice', days: 2, speed: 'rapid' },
        { account: 'alice', rated: null },
        { account: 'nobody' },
      ]
      for (const query of queries) value(await s.run('store.insights.insights', [query]))
    } finally {
      await s.close()
    }
  })
})

/* ── Usage ── */

describe('usage', () => {
  it('adds flushed counters to the stored totals, reports them and resets them', async () => {
    const s = await open()
    try {
      value(await s.run('store.usage.usageReport', []))
      value(
        await s.run('store.usage.flushUsage', [
          [
            { account: 'alice', kind: 'games', requests: 2, bytesIn: 100 },
            { account: 'alice', kind: 'games', requests: 1, bytesIn: 5 },
            { account: 'bob', kind: 'other', requests: 1, bytesIn: 0 },
          ],
        ]),
      )
      seedGame(s, 'UsageGame1', 'alice')
      s.seed(
        "INSERT INTO api_cache (key, value, fetchedAt) VALUES ('alice:games', 'cached', 1), ('bob:x', 'longer cached value', 1), ('plain', 'abc', 1)",
      )
      value(await s.run('store.usage.usageReport', []))
      value(await s.run('store.usage.flushUsage', [[]]))
      value(await s.run('store.usage.resetUsage', []))
      value(await s.run('store.usage.usageReport', []))
    } finally {
      await s.close()
    }
  })
})

/* ── Voice log ── */

describe('voice log', () => {
  const attempt = (overrides: Record<string, unknown> = {}) => ({
    source: 'computer',
    heard: 'knight f three',
    confidence: 0.9,
    words: [{ word: 'knight', conf: 0.95 }],
    outcome: 'played',
    ...overrides,
  })

  it('saves, updates, lists and exports attempts newest first, and clears them', async () => {
    const s = await open()
    try {
      const first = value<number>(
        await s.run('store.voiceLog.saveVoiceAttempt', [attempt({ heard: 'e two to e four' })]),
      )
      const second = value<number>(
        await s.run('store.voiceLog.saveVoiceAttempt', [
          attempt({ parsed: 'Nf3', expected: 'Nf3', fen: START, retryOf: first }),
        ]),
      )
      expect(second).toBeGreaterThan(first)
      value(
        await s.run('store.voiceLog.updateVoiceAttempt', [
          first,
          { outcome: 'confirmed', expected: 'Nf3' },
        ]),
      )
      value(
        await s.run('store.voiceLog.updateVoiceAttempt', [first, { outcome: '', expected: '' }]),
      )
      value(await s.run('store.voiceLog.updateVoiceAttempt', [999, { outcome: 'ignored' }]))
      expect(value<{ id: number }[]>(await s.run('store.voiceLog.voiceHistory', [10]))[0]!.id).toBe(
        second,
      )
      value(await s.run('store.voiceLog.voiceHistory', [0]))
      value(
        await s.run('store.voiceLog.saveVoiceAttempt', [
          attempt({ heard: 'castle', words: 'not a list' }),
        ]),
      )
      s.seed("UPDATE voice_log SET words = '{broken' WHERE id = ?", [first])
      value(await s.run('store.voiceLog.voiceHistory', [10]))
      const document = value<string>(await s.run('store.voiceLog.voiceHistoryDocument', []))
      expect(JSON.parse(document)).toHaveProperty('entries')
      value(await s.run('store.voiceLog.clearVoiceHistory', []))
      expect(value(await s.run('store.voiceLog.voiceHistory', [10]))).toEqual([])
      value(await s.run('store.voiceLog.voiceHistoryDocument', []))
    } finally {
      await s.close()
    }
  })

  it('keeps only the newest 5000 entries', async () => {
    const s = await open()
    try {
      for (let n = 0; n < 5_005; n++)
        value(
          await s.run('store.voiceLog.saveVoiceAttempt', [attempt({ heard: `word ${n}` })], false),
        )
      s.compare()
      expect(
        value<unknown[]>(await s.run('store.voiceLog.voiceHistory', [10_000], false)),
      ).toHaveLength(5_000)
    } finally {
      await s.close()
    }
  }, 120_000)
})

/* ── Position lookups ── */

describe('position lookup cache', () => {
  const entry = (fetchedAt: number, fen = START) => ({
    kind: 'masters',
    fen,
    fetchedAt,
    total: 0,
    moves: [],
  })

  it('stores and reads entries as text, keeps the newest 256 and reads nothing too large', async () => {
    const s = await open()
    try {
      value(await s.run('store.setupPositionLookup.write', ['a', entry(1)]))
      value(await s.run('store.setupPositionLookup.write', ['b', entry(2, 'x')]))
      value(await s.run('store.setupPositionLookup.read', ['a']))
      value(await s.run('store.setupPositionLookup.read', ['missing']))
      value(
        await s.run('store.setupPositionLookup.write', [
          'huge',
          { ...entry(3), moves: [{ san: 'x'.repeat(600_000) }] },
        ]),
      )
      value(await s.run('store.setupPositionLookup.read', ['huge']))
      for (let n = 0; n < 260; n++)
        value(await s.run('store.setupPositionLookup.write', [`k${n}`, entry(100 + n)], false))
      s.compare()
      value(await s.run('store.setupPositionLookup.read', ['a']))
    } finally {
      await s.close()
    }
  })
})

/* ── Coverage ── */

it('names every method the Rust modules export once', () => {
  const methods = [
    'store.library.library',
    'store.library.readSession',
    'store.library.importLibrary',
    'store.library.studyCommand',
    'store.library.saveArchivedGame',
    'store.library.removeArchivedGame',
    'store.library.addMistakes',
    'store.library.answerMistake',
    'store.library.saveSession',
    'store.library.joinedTournaments',
    'store.library.rememberTournament',
    'store.library.forgetTournament',
    'store.library.forgetTournamentsOf',
    'store.library.recordRepertoireMiss',
    'store.library.clearRepertoireMisses',
    'store.reviewStore.readReview',
    'store.reviewStore.writeReview',
    'store.reviewStore.reviewSummaries',
    'store.reviewStore.hasAccount',
    'store.reviewStore.markChecked',
    'store.reviewStore.gamesToReview',
    'store.reviewStore.reviewCount',
    'store.runs.runSummary',
    'store.runs.saveRun',
    'store.runs.clearRuns',
    'store.insights.insights',
    'store.usage.flushUsage',
    'store.usage.resetUsage',
    'store.usage.usageReport',
    'store.voiceLog.saveVoiceAttempt',
    'store.voiceLog.updateVoiceAttempt',
    'store.voiceLog.voiceHistory',
    'store.voiceLog.clearVoiceHistory',
    'store.voiceLog.voiceHistoryDocument',
    'store.setupPositionLookup.read',
    'store.setupPositionLookup.write',
  ]
  expect(new Set(methods).size).toBe(methods.length)
})
