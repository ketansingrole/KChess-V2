import { describe, expect, it, vi } from 'vitest'
import { ChallengeInbox, readChallengeEvent } from '../../src/services/challenges'

const challenge = (overrides: Record<string, unknown> = {}) => ({
  type: 'challenge',
  challenge: {
    id: 'AbCd1234',
    status: 'created',
    challenger: { id: 'bob', name: 'Bob', rating: 1700 },
    destUser: { id: 'alice', name: 'Alice', rating: 1650 },
    variant: { key: 'standard', name: 'Standard' },
    rated: true,
    speed: 'rapid',
    timeControl: { type: 'clock', limit: 600, increment: 5 },
    color: 'white',
    ...overrides,
  },
  compat: { board: true },
})

describe('challenge inbox', () => {
  it('reads incoming and outgoing challenges from either account’s point of view', () => {
    const incoming = readChallengeEvent('Alice', challenge())!
    expect(incoming.info).toMatchObject({
      direction: 'in',
      opponent: { name: 'Bob', rating: 1700 },
      timeControl: { type: 'clock', limit: 600, increment: 5 },
      playable: true,
    })
    expect(readChallengeEvent('Bob', challenge())!.info.direction).toBe('out')
    expect(readChallengeEvent('Alice', { type: 'gameStart' })).toBeUndefined()
  })

  it('marks what KChess cannot play', () => {
    const house = readChallengeEvent(
      'Alice',
      challenge({ variant: { key: 'crazyhouse', name: 'Crazyhouse' } }),
    )!.info
    expect(house.playable).toBe(false)
    expect(house.problem).toContain('Crazyhouse')
    const bullet = readChallengeEvent('Alice', { ...challenge(), compat: { board: false } })!.info
    expect(bullet.playable).toBe(false)
  })

  it('rejects malformed events and oversized names', () => {
    expect(readChallengeEvent('Alice', challenge({ id: '../../x' }))).toBeUndefined()
    expect(
      readChallengeEvent('Alice', challenge({ challenger: { id: 'b', name: 'x'.repeat(200) } })),
    ).toBeUndefined()
  })

  it('announces a challenge once, removes it when cancelled and forgets stale ones', () => {
    let now = 1_000
    const changed = vi.fn()
    const inbox = new ChallengeInbox(changed, () => now)
    expect(inbox.ingest('Alice', challenge())?.id).toBe('AbCd1234')
    // Lichess repeats pending challenges after a reconnect: not new again.
    expect(inbox.ingest('Alice', challenge())).toBeUndefined()
    expect(inbox.list()).toHaveLength(1)
    inbox.ingest('Alice', { ...challenge(), type: 'challengeCanceled' })
    expect(inbox.list()).toHaveLength(0)
    inbox.ingest('Alice', challenge())
    now += 31 * 60_000
    expect(inbox.list()).toHaveLength(0)
    inbox.ingest('Alice', challenge({ timeControl: { type: 'correspondence', daysPerTurn: 3 } }))
    now += 3 * 86_400_000
    expect(inbox.list()).toHaveLength(1)
    inbox.forget('alice')
    expect(inbox.list()).toHaveLength(0)
    expect(changed).toHaveBeenCalled()
  })
})
