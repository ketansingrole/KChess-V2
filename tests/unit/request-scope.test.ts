import { expect, it, vi } from 'vitest'
import * as fc from 'fast-check'
import { RequestScope, SubscriptionScope } from '../../src/shared/requestScope'

it('aborts superseded work and rejects late replies across arbitrary invalidation sequences', () => {
  fc.assert(
    fc.property(fc.array(fc.boolean(), { minLength: 1, maxLength: 40 }), (actions) => {
      const scope = new RequestScope()
      const pending: ReturnType<RequestScope['capture']>[] = []
      for (const start of actions) {
        if (start) pending.push(scope.next())
        else scope.invalidate()
        for (const request of pending.slice(0, start ? -1 : undefined)) {
          expect(request.current()).toBe(false)
          expect(request.signal.aborted).toBe(true)
        }
        if (start) expect(pending.at(-1)?.current()).toBe(true)
      }
      scope.invalidate()
      expect(pending.every((request) => request.signal.aborted && !request.current())).toBe(true)
    }),
  )
})
it('does not duplicate listeners and releases every listener even if a cleanup fails', () => {
  const scope = new SubscriptionScope()
  const off = vi.fn()
  const subscribe = vi.fn(() => off)
  scope.attach([subscribe])
  scope.attach([subscribe])
  expect(subscribe).toHaveBeenCalledTimes(1)
  scope.detach()
  scope.detach()
  expect(off).toHaveBeenCalledTimes(1)
  scope.attach([
    () => () => {
      throw new Error('cleanup failed')
    },
    subscribe,
  ])
  expect(() => scope.detach()).toThrow('cleanup failed')
  expect(off).toHaveBeenCalledTimes(2)
})
it('rolls back partially attached listeners if subscription fails', () => {
  const scope = new SubscriptionScope()
  const off = vi.fn()
  expect(() =>
    scope.attach([
      () => off,
      () => {
        throw new Error('subscribe failed')
      },
    ]),
  ).toThrow('subscribe failed')
  expect(off).toHaveBeenCalledTimes(1)
  scope.detach()
  expect(off).toHaveBeenCalledTimes(1)
})
