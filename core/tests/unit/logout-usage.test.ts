import { it, expect, vi } from 'vitest'
import { closeNativeCore } from '../../src/services/nativeCore'
import { useTestDatabase, type TestDatabase } from '../../../tests/fixtures/nativeStore'
import { deferred } from '../../../tests/fixtures/deferred'
import { withUsage, meteredFetch, forgetUsage, flushUsage } from '../../src/services/usage'

const state = vi.hoisted(() => ({ db: null as TestDatabase | null }))

it('does not restore account usage from a request that finishes after logout', async () => {
  state.db = useTestDatabase()
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
    await closeNativeCore()
    state.db = null
  }
})
