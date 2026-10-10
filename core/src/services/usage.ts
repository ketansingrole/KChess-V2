import { scopedState, coreSignal, platform } from './platform'
import { AsyncLocalStorage } from 'node:async_hooks'
import { stat } from 'node:fs/promises'
import { join } from 'node:path'
import { logDebug, logWarn } from './logger'
import { nativeCallSync } from './nativeCore'
import type { UsageCell, UsageKind, UsageReport } from '../contracts/types'

/**
 * Network accounting for everything KChess asks of Lichess. Requests carry the
 * account and purpose they were made for in an async context, so each byte can
 * be attributed. The counters are batched in memory here and written to the Rust
 * core's `kchess.db` (`crates/kchess-core/src/store/usage.rs`), so they survive restarts.
 */
interface Context {
  account: string
  kind: UsageKind
  epoch?: number
}

const context = new AsyncLocalStorage<Context>()
const UNATTRIBUTED: Context = { account: '', kind: 'other' }

/** Run `work` so every Lichess request it makes is counted against `account`. */
export function withUsage<T>(account: string, kind: UsageKind, work: () => T): T {
  return context.run(
    {
      account: account.toLowerCase(),
      kind,
      epoch: serviceState.accountEpochs.get(account.toLowerCase()) ?? 0,
    },
    work,
  )
}

/**
 * Count the rest of the current async flow (including streams it opens) against `account`.
 * For long-lived sessions where the account is only known midway through a call.
 */
export function attributeTo(account: string, kind: UsageKind): void {
  context.enterWith({
    account: account.toLowerCase(),
    kind,
    epoch: serviceState.accountEpochs.get(account.toLowerCase()) ?? 0,
  })
}

function record(ctx: Context, requests: number, bytesIn: number): void {
  if (coreSignal()?.aborted) return
  if ((ctx.epoch ?? 0) !== (serviceState.accountEpochs.get(ctx.account) ?? 0)) return
  const key = `${ctx.account}|${ctx.kind}`
  const cell = serviceState.pending.get(key) ?? {
    account: ctx.account,
    kind: ctx.kind,
    requests: 0,
    bytesIn: 0,
  }
  cell.requests += requests
  cell.bytesIn += bytesIn
  serviceState.pending.set(key, cell)
  // Streams report every chunk; write to disk at most every few seconds.
  serviceState.flushTimer ??= setTimeout(() => {
    serviceState.flushTimer = undefined
    flushUsage()
  }, 5000)
  serviceState.flushTimer.unref()
}

export function recordUsage(
  account: string,
  kind: UsageKind,
  requests: number,
  bytesIn: number,
): void {
  record(
    {
      account: account.toLowerCase(),
      kind,
      epoch: serviceState.accountEpochs.get(account.toLowerCase()) ?? 0,
    },
    requests,
    bytesIn,
  )
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
  if (!serviceState.pending.size) return
  const rows = [...serviceState.pending.values()]
  serviceState.pending.clear()
  try {
    nativeCallSync('store.usage.flushUsage', rows)
  } catch (cause) {
    logWarn('usage', 'Dropping usage batch:', rows.length, cause)
  }
}

/** Ignore traffic still arriving from requests started before logout. */
export function forgetUsage(accounts: string[]): void {
  for (const name of accounts) {
    const account = name.toLowerCase()
    serviceState.accountEpochs.set(account, (serviceState.accountEpochs.get(account) ?? 0) + 1)
    for (const [key, cell] of serviceState.pending)
      if (cell.account === account) serviceState.pending.delete(key)
  }
}

export function resetUsage(): void {
  serviceState.pending.clear()
  nativeCallSync('store.usage.resetUsage')
}

async function fileSize(path: string): Promise<number> {
  try {
    return (await stat(path)).size
  } catch (cause) {
    logDebug('usage', 'Size check failed:', path, cause)
    return 0
  }
}

/** What each account has downloaded from Lichess and what it occupies on this computer. */
export async function usageReport(): Promise<UsageReport> {
  flushUsage()
  const report = nativeCallSync<Omit<UsageReport, 'dbBytes'>>('store.usage.usageReport')
  const path = join(platform().dataDir, 'kchess.db')
  const dbBytes =
    (await fileSize(path)) +
    (await fileSize(`${path}-wal`)) +
    (await fileSize(path.replace(/kchess\.db$/, 'puzzles.db'))) +
    (await fileSize(path.replace(/kchess\.db$/, 'puzzles.db-wal')))
  return { ...report, dbBytes }
}

export function closeUsage(): void {
  clearTimeout(serviceState.flushTimer)
  serviceState.flushTimer = undefined
  serviceState.pending.clear()
  serviceState.accountEpochs.clear()
}

const serviceState = scopedState(() => ({
  accountEpochs: new Map<string, number>(),
  pending: new Map<string, UsageCell & { account: string; kind: UsageKind }>(),
  flushTimer: undefined as NodeJS.Timeout | undefined,
}))
