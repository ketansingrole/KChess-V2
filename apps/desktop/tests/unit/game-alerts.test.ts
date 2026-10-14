import { describe, expect, it } from 'vitest'
import { redundantGameAlert } from '../../app/utils/gameAlerts'

describe('game alerts', () => {
  it('drops alerts about the game on screen', () => {
    expect(redundantGameAlert('opponentMove', 'online', true)).toBe(true)
    expect(redundantGameAlert('gameEvents', 'online', true)).toBe(true)
    expect(redundantGameAlert('lowTime', 'online', true)).toBe(true)
    expect(redundantGameAlert('computerMove', 'computer', true)).toBe(true)
  })

  it('keeps them on another page or in the background', () => {
    expect(redundantGameAlert('opponentMove', 'studies', true)).toBe(false)
    expect(redundantGameAlert('computerMove', 'online', true)).toBe(false)
    expect(redundantGameAlert('opponentMove', 'online', false)).toBe(false)
  })

  it('always keeps challenges and test notifications', () => {
    expect(redundantGameAlert('challenge', 'online', true)).toBe(false)
    expect(redundantGameAlert('test', 'settings', true)).toBe(false)
  })
})
