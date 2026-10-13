import { describe, expect, it } from 'vitest'
import { TV_CHANNEL_KEYS, TV_CHANNELS } from '@kchess/rules/tvChannels'

/** Lichess TV channels, keyed as the feed URLs name them. */
describe('tv channels', () => {
  it('lists every channel key the feed URLs use', () => {
    expect(TV_CHANNEL_KEYS).toEqual([
      'best',
      'bullet',
      'blitz',
      'rapid',
      'classical',
      'ultraBullet',
      'chess960',
      'kingOfTheHill',
      'threeCheck',
      'antichess',
      'atomic',
      'horde',
      'racingKings',
      'crazyhouse',
      'bot',
      'computer',
    ])
  })

  it('derives the keys from the channels', () => {
    expect(TV_CHANNELS).toHaveLength(16)
    expect(TV_CHANNELS.map((channel) => channel.key)).toEqual(TV_CHANNEL_KEYS)
  })

  it('labels every channel for the picker', () => {
    const labels = new Map(TV_CHANNELS.map((channel) => [channel.key, channel.label]))
    expect(labels.get('best')).toBe('Top rated')
    expect(labels.get('ultraBullet')).toBe('UltraBullet')
    expect(labels.get('kingOfTheHill')).toBe('King of the Hill')
    expect(labels.get('threeCheck')).toBe('Three-check')
    expect(labels.get('racingKings')).toBe('Racing Kings')
    expect(labels.get('bot')).toBe('Bots')
    for (const channel of TV_CHANNELS) {
      expect(channel.key).toMatch(/^[a-zA-Z0-9]+$/)
      expect(channel.label.length).toBeGreaterThan(0)
    }
    expect(new Set(TV_CHANNEL_KEYS).size).toBe(TV_CHANNEL_KEYS.length)
  })
})
