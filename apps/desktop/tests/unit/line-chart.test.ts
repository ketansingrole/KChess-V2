import { describe, expect, it } from 'vitest'
import { areaPath, linePath, linearScale, nearestIndex, niceTicks } from '../../app/utils/lineChart'

const identity = (value: number) => value

describe('line chart geometry', () => {
  it('scales linearly and centres an empty domain', () => {
    const scale = linearScale([0, 10], [100, 0])
    expect(scale(0)).toBe(100)
    expect(scale(5)).toBe(50)
    expect(linearScale([3, 3], [0, 40])(3)).toBe(20)
  })

  it('picks round ticks inside the domain', () => {
    expect(niceTicks([1185, 1615], 4)).toEqual([1200, 1300, 1400, 1500, 1600])
    expect(niceTicks([0, 1], 4)).toEqual([0, 0.2, 0.4, 0.6, 0.8, 1])
    expect(niceTicks([0.1, 0.3], 4)).toEqual([0.1, 0.15, 0.2, 0.25, 0.3])
    expect(niceTicks([5, 5], 4)).toEqual([])
    expect(niceTicks([0, Number.POSITIVE_INFINITY], 4)).toEqual([])
  })

  it('draws two points as a straight line and more as a monotone curve', () => {
    expect(linePath([0, 10], [0, 10], identity, identity)).toBe('M0,0L10,10')
    const curve = linePath([0, 10, 20], [0, 10, 10], identity, identity)
    expect(curve.startsWith('M0,0C')).toBe(true)
    // A flat stretch has a flat tangent at its start, so the curve never overshoots it.
    const controls = [...curve.matchAll(/(-?[\d.]+),(-?[\d.]+)/g)].map((match) => Number(match[2]))
    expect(Math.max(...controls)).toBeLessThanOrEqual(10)
  })

  it('breaks the line at missing values and closes areas to the baseline', () => {
    expect(linePath([0, 1, 2, 3], [5, undefined, 7, 8], identity, identity)).toBe('M0,5M2,7L3,8')
    expect(areaPath([0, 1], [5, 6], identity, identity, 20)).toBe('M0,5L1,6L1,20L0,20Z')
    expect(linePath([0, 1], [undefined, undefined], identity, identity)).toBe('')
  })

  it('finds the nearest x', () => {
    const xs = [0, 10, 20, 40]
    expect(nearestIndex(xs, -5)).toBe(0)
    expect(nearestIndex(xs, 14)).toBe(1)
    expect(nearestIndex(xs, 16)).toBe(2)
    expect(nearestIndex(xs, 31)).toBe(3)
    expect(nearestIndex(xs, 99)).toBe(3)
    expect(nearestIndex([], 1)).toBe(-1)
  })
})
