import { expect, it } from 'vitest'
import { boardResult, pgnResult, timeoutWinner } from '../../src/shared/gameResult'
import { positionAfter, positionFromFen, setupPositionAfter } from '../../src/shared/chess'

it('names how the board ended the game, including variant wins', () => {
  const mate = positionAfter(['f2f3', 'e7e5', 'g2g4', 'd8h4'])
  expect(boardResult(mate, 'standard')).toEqual({ winner: 'black', reason: 'Checkmate' })
  const stalemate = positionFromFen('7k/5Q2/6K1/8/8/8/8/8 b - - 0 1')!
  expect(boardResult(stalemate, 'standard')).toEqual({ reason: 'Stalemate' })
  const hill = setupPositionAfter(
    { variant: 'kingOfTheHill', fen: '7k/8/8/8/8/8/3K4/8 w - - 0 1' },
    ['d2d3', 'h8g8', 'd3d4'],
  )
  expect(boardResult(hill, 'kingOfTheHill')).toEqual({
    winner: 'white',
    reason: 'King reached the centre',
  })
  expect(boardResult(positionAfter([]), 'standard', 'Threefold repetition')).toEqual({
    reason: 'Threefold repetition',
  })
  expect(boardResult(positionAfter([]), 'standard')).toBeNull()
})

it('draws a timeout when the other side cannot possibly mate', () => {
  const bareKing = positionFromFen('7k/8/8/8/8/8/8/KQ6 w - - 0 1')!
  expect(timeoutWinner(bareKing, 'white')).toBeUndefined()
  expect(timeoutWinner(bareKing, 'black')).toBe('white')
  expect(pgnResult(false, 'white')).toBe('*')
  expect(pgnResult(true, 'black')).toBe('0-1')
  expect(pgnResult(true)).toBe('1/2-1/2')
})
