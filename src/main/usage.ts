import { AsyncLocalStorage } from 'node:async_hooks'
import { stat } from 'node:fs/promises'
import { dbPath, getDb } from './db'
import type { UsageCell, UsageKind, UsageReport } from '../shared/types'

/**
 * Network accounting for everything KChess asks of Lichess. Requests carry the
 * account and purpose they were made for in an async context, so each byte can
 * be attributed; the totals are kept in SQLite so they survive restarts.
 */
interface Context {
  account: string
  kind: UsageKind
}

const context = new AsyncLocalStorage<Context>()
const UNATTRIBUTED: Context = { account: '', kind: 'other' }

/** Run `work` so every Lichess request it makes is counted against `account`. */
export function withUsage<T>(account: string, kind: UsageKind, work: () => T): T {
  return context.run({ account: account.toLowerCase(), kind }, work)
}

/**
 * Count the rest of the current async flow (including streams it opens) against `account`.
 * For long-lived sessions where the account is only known midway through a call.
 */
export function attributeTo(account: string, kind: UsageKind): void {
  context.enterWith({ account: account.toLowerCase(), kind })
}

const pending = new Map<string, UsageCell & { account: string; kind: UsageKind }>()
let flushTimer: NodeJS.Timeout | undefined

function record(ctx: Context, requests: number, bytesIn: number): void {
  const key = `${ctx.account}|${ctx.kind}`
  const cell = pending.get(key) ?? { account: ctx.account, kind: ctx.kind, requests: 0, bytesIn: 0 }
  cell.requests += requests
  cell.bytesIn += bytesIn
  pending.set(key, cell)
  // Streams report every chunk; write to disk at most every few seconds.
  flushTimer ??= setTimeout(() => {
    flushTimer = undefined
    flushUsage()
  }, 5000)
  flushTimer.unref()
}

export function recordUsage(
  account: string,
  kind: UsageKind,
  requests: number,
  bytesIn: number,
): void {
  record({ account, kind }, requests, bytesIn)
}

/** `fetch` that counts requests and the (decompressed) bytes of every response body. */
export const meteredFetch: typeof fetch = async (input, init) => {
  const ctx = context.getStore() ?? UNATTRIBUTED
  const response = await fetch(input, init)
  record(ctx, 1, 0)
  if (!response.body) return response
  const counter = new TransformStream<Uint8Array, Uint8Array>({
    transform(chunk, controller) {
      record(ctx, 0, chunk.byteLength)
      controller.enqueue(chunk)
    },
  })
  const wrapped = new Response(response.body.pipeThrough(counter), {
    status: response.status,
    statusText: response.statusText,
    headers: response.headers,
  })
  // Callers read the URL of the response (for error messages); a rebuilt Response has none.
  Object.defineProperty(wrapped, 'url', { value: response.url })
  return wrapped
}

/** Write the counters accumulated in memory to the database. */
export function flushUsage(): void {
  if (!pending.size) return
  const rows = [...pending.values()]
  pending.clear()
  try {
    const database = getDb()
    const upsert = database.prepare(
      `INSERT INTO usage (account, kind, requests, bytesIn, since) VALUES (?, ?, ?, ?, ?)
       ON CONFLICT(account, kind) DO UPDATE SET requests = requests + excluded.requests, bytesIn = bytesIn + excluded.bytesIn`,
    )
    database.exec('BEGIN IMMEDIATE')
    try {
      const now = Date.now()
      for (const row of rows) upsert.run(row.account, row.kind, row.requests, row.bytesIn, now)
      database.exec('COMMIT')
    } catch (cause) {
      database.exec('ROLLBACK')
      throw cause
    }
  } catch {
    // Accounting must never break the app; the counts of this batch are simply lost.
  }
}

export function resetUsage(): void {
  pending.clear()
  getDb().exec('DELETE FROM usage')
}

async function fileSize(path: string): Promise<number> {
  try {
    return (await stat(path)).size
  } catch {
    return 0
  }
}

/** What each account has downloaded from Lichess and what it occupies on this computer. */
export async function usageReport(): Promise<UsageReport> {
  flushUsage()
  const database = getDb()
  const report: UsageReport = { accounts: {}, storage: {}, dbBytes: 0 }

  const usageRows = database
    .prepare('SELECT account, kind, requests, bytesIn, since FROM usage')
    .all() as unknown as {
    account: string
    kind: UsageKind
    requests: number
    bytesIn: number
    since: number
  }[]
  for (const row of usageRows) {
    const entry = (report.accounts[row.account] ??= {
      total: { requests: 0, bytesIn: 0 },
      byKind: {},
    })
    entry.byKind[row.kind] = { requests: row.requests, bytesIn: row.bytesIn }
    entry.total.requests += row.requests
    entry.total.bytesIn += row.bytesIn
    report.since = Math.min(report.since ?? Infinity, row.since)
  }

  const gameRows = database
    .prepare(
      `SELECT account, COUNT(*) AS games,
        SUM(LENGTH(id) + LENGTH(status) + LENGTH(speed) + LENGTH(perf) + LENGTH(opponent)
          + COALESCE(LENGTH(opening), 0) + LENGTH(moves) + COALESCE(LENGTH(pgn), 0) + 64) AS bytes
       FROM games GROUP BY account`,
    )
    .all() as unknown as { account: string; games: number; bytes: number }[]
  for (const row of gameRows)
    report.storage[row.account.toLowerCase()] = {
      games: row.games,
      bytes: row.bytes,
      cacheBytes: 0,
    }

  const cacheRows = database
    .prepare('SELECT key, LENGTH(value) AS bytes FROM api_cache')
    .all() as unknown as { key: string; bytes: number }[]
  for (const row of cacheRows) {
    const account = row.key.split(':')[0] ?? ''
    const entry = (report.storage[account] ??= { games: 0, bytes: 0, cacheBytes: 0 })
    entry.cacheBytes += row.bytes
  }

  const path = dbPath()
  report.dbBytes =
    (await fileSize(path)) +
    (await fileSize(`${path}-wal`)) +
    (await fileSize(path.replace(/kchess\.db$/, 'puzzles.db'))) +
    (await fileSize(path.replace(/kchess\.db$/, 'puzzles.db-wal')))
  return report
}
