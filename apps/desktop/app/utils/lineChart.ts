/** Geometry for the shared line chart: scales, ticks, curve paths and pointer lookup. */

export type Scale = (value: number) => number

/** Maps `domain` linearly onto `range`; an empty domain maps to the middle of the range. */
export function linearScale(
  [d0, d1]: readonly [number, number],
  [r0, r1]: readonly [number, number],
): Scale {
  const span = d1 - d0
  if (!span) return () => (r0 + r1) / 2
  return (value) => r0 + ((value - d0) / span) * (r1 - r0)
}

/** About `count` round values (1, 2 or 5 × 10ⁿ apart) inside the domain. */
export function niceTicks([lo, hi]: readonly [number, number], count: number): number[] {
  if (!(hi > lo) || count < 1 || !Number.isFinite(lo) || !Number.isFinite(hi)) return []
  const raw = (hi - lo) / count
  let step = 10 ** Math.floor(Math.log10(raw))
  const ratio = raw / step
  if (ratio >= 7.5) step *= 10
  else if (ratio >= 3.5) step *= 5
  else if (ratio >= 1.5) step *= 2
  const ticks: number[] = []
  // Integer bounds with a little tolerance, so 6 × 0.05 still counts as inside [0.1, 0.3].
  const last = Math.floor(hi / step + 1e-9)
  for (let index = Math.ceil(lo / step - 1e-9); index <= last; index++)
    // Multiplying the index keeps decimal steps free of accumulated error.
    ticks.push(Number((index * step).toPrecision(12)))
  return ticks
}

type Point = readonly [number, number]

/** Tangent at the middle of three points, kept monotone (Fritsch–Carlson). */
function middleSlope(p0: Point, p1: Point, p2: Point): number {
  const h0 = p1[0] - p0[0]
  const h1 = p2[0] - p1[0]
  const s0 = h0 ? (p1[1] - p0[1]) / h0 : 0
  const s1 = h1 ? (p2[1] - p1[1]) / h1 : 0
  const weighted = h0 + h1 ? (s0 * h1 + s1 * h0) / (h0 + h1) : 0
  return (
    (Math.sign(s0) + Math.sign(s1)) *
      Math.min(Math.abs(s0), Math.abs(s1), 0.5 * Math.abs(weighted)) || 0
  )
}

/** Tangent at an end point, from its neighbour's tangent. */
function endSlope(from: Point, to: Point, neighbour: number): number {
  const h = to[0] - from[0]
  return h ? ((3 * (to[1] - from[1])) / h - neighbour) / 2 : neighbour
}

const fixed = (n: number): string => String(Math.round(n * 100) / 100)

/** One run of points as a monotone-x cubic path (d3's `curveMonotoneX`), no overshoot. */
function runPath(points: readonly Point[]): string {
  const [first, second] = points
  if (!first) return ''
  const start = `M${fixed(first[0])},${fixed(first[1])}`
  if (!second) return start
  if (points.length === 2) return `${start}L${fixed(second[0])},${fixed(second[1])}`
  const slopes = points.map((point, index) => {
    const before = points[index - 1]
    const after = points[index + 1]
    return before && after ? middleSlope(before, point, after) : 0
  })
  slopes[0] = endSlope(first, second, slopes[1] ?? 0)
  const [beforeLast, last] = points.slice(-2)
  if (beforeLast && last)
    slopes[points.length - 1] = endSlope(beforeLast, last, slopes[points.length - 2] ?? 0)
  let path = start
  let [x0, y0] = first
  points.slice(1).forEach(([x1, y1], index) => {
    const dx = (x1 - x0) / 3
    path +=
      `C${fixed(x0 + dx)},${fixed(y0 + dx * (slopes[index] ?? 0))}` +
      ` ${fixed(x1 - dx)},${fixed(y1 - dx * (slopes[index + 1] ?? 0))} ${fixed(x1)},${fixed(y1)}`
    ;[x0, y0] = [x1, y1]
  })
  return path
}

/** Defined points split into runs; a missing value breaks the line. */
function runs(
  xs: readonly number[],
  values: readonly (number | undefined)[],
  x: Scale,
  y: Scale,
): Point[][] {
  const result: Point[][] = []
  let current: Point[] = []
  xs.forEach((xValue, index) => {
    const value = values[index]
    if (value === undefined || !Number.isFinite(value)) {
      if (current.length) result.push(current)
      current = []
    } else current.push([x(xValue), y(value)])
  })
  if (current.length) result.push(current)
  return result
}

export function linePath(
  xs: readonly number[],
  values: readonly (number | undefined)[],
  x: Scale,
  y: Scale,
): string {
  return runs(xs, values, x, y).map(runPath).join('')
}

/** The line's runs closed down to `baseline` (a pixel y), for a filled area under them. */
export function areaPath(
  xs: readonly number[],
  values: readonly (number | undefined)[],
  x: Scale,
  y: Scale,
  baseline: number,
): string {
  return runs(xs, values, x, y)
    .map((run) => {
      const first = run[0]
      const last = run.at(-1)
      if (!first || !last) return ''
      return `${runPath(run)}L${fixed(last[0])},${fixed(baseline)}L${fixed(first[0])},${fixed(baseline)}Z`
    })
    .join('')
}

/** Index of the x value nearest `target`; `xs` is ascending. -1 when empty. */
export function nearestIndex(xs: readonly number[], target: number): number {
  if (!xs.length) return -1
  let lo = 0
  let hi = xs.length - 1
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1
    if ((xs[mid] ?? target) <= target) lo = mid
    else hi = mid
  }
  return Math.abs((xs[hi] ?? target) - target) < Math.abs(target - (xs[lo] ?? target)) ? hi : lo
}
