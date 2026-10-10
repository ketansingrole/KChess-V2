import { expect, it, vi } from 'vitest'
import { RequestScope } from '@kchess/rules/requestScope'
import { requestInsights } from '../../app/utils/insightsRequest'
import type { InsightsReport } from '@kchess/contracts/types'
import { deferred } from './fixtures'

const report = (total: number) => ({ total }) as InsightsReport
it('keeps the newest result when filter replies arrive out of order', async () => {
  const scope = new RequestScope()
  const old = deferred<InsightsReport>()
  const fresh = deferred<InsightsReport>()
  const read = vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(fresh.promise)
  const first = requestInsights(scope, { account: 'Alice' }, new AbortController().signal, read)
  const rejected = expect(first).rejects.toMatchObject({ name: 'AbortError' })
  const second = requestInsights(scope, { account: 'Bob' }, new AbortController().signal, read)
  fresh.resolve(report(2))
  await expect(second).resolves.toEqual(report(2))
  old.resolve(report(1))
  await rejected
})
it('does not let an old abort signal cancel the current request', async () => {
  const scope = new RequestScope()
  const old = deferred<InsightsReport>()
  const fresh = deferred<InsightsReport>()
  const controller = new AbortController()
  const first = requestInsights(scope, { account: 'Alice' }, controller.signal, () => old.promise)
  const rejected = expect(first).rejects.toMatchObject({ name: 'AbortError' })
  const second = requestInsights(
    scope,
    { account: 'Bob' },
    new AbortController().signal,
    () => fresh.promise,
  )
  controller.abort()
  fresh.resolve(report(2))
  await expect(second).resolves.toEqual(report(2))
  old.reject(new Error('Old failure'))
  await rejected
})
it('rejects late output after unmount and does not start an already aborted request', async () => {
  const scope = new RequestScope()
  const pending = deferred<InsightsReport>()
  const first = requestInsights(
    scope,
    { account: 'Alice' },
    new AbortController().signal,
    () => pending.promise,
  )
  const rejected = expect(first).rejects.toMatchObject({ name: 'AbortError' })
  scope.invalidate()
  pending.resolve(report(1))
  await rejected
  const controller = new AbortController()
  controller.abort()
  const read = vi.fn()
  await expect(
    requestInsights(scope, { account: 'Alice' }, controller.signal, read),
  ).rejects.toMatchObject({ name: 'AbortError' })
  expect(read).not.toHaveBeenCalled()
})
