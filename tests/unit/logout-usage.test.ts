import { it, expect, vi } from 'vitest'
import { DatabaseSync } from 'node:sqlite'
import { deferred } from './fixtures'
import { withUsage, meteredFetch, forgetUsage, flushUsage } from '../../src/core/usage'

const state = vi.hoisted(() => ({ db: null as DatabaseSync | null }))
vi.mock('../../src/core/db', () => ({ getDb: () => state.db, dbPath: () => '' }))

it('does not restore account usage from a request that finishes after logout', async () => {
  state.db = new DatabaseSync(':memory:')
  state.db.exec(
    'CREATE TABLE usage (account TEXT, kind TEXT, requests INTEGER, bytesIn INTEGER, since INTEGER, PRIMARY KEY(account, kind))',
  )
  const response = deferred<Response>()
  vi.stubGlobal(
    'fetch',
    vi.fn(() => response.promise),
  )
  try {
    const old = withUsage('Alice', 'games', () => meteredFetch('https://lichess.org/api/test'))
    forgetUsage(['ALICE'])
    response.resolve(new Response('old account data'))
    await (await old).text()
    flushUsage()
    expect(state.db.prepare('SELECT * FROM usage').all()).toEqual([])
    await withUsage('Alice', 'games', async () =>
      (await meteredFetch('https://lichess.org/api/test')).text(),
    )
    flushUsage()
    expect(state.db.prepare('SELECT account, requests FROM usage').all()).toEqual([
      { account: 'alice', requests: 1 },
    ])
  } finally {
    state.db.close()
    state.db = null
  }
})
