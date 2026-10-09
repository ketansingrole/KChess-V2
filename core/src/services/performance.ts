import { monitorEventLoopDelay, performance } from 'node:perf_hooks'
const samples = new Map<string, number[]>()
const delay = monitorEventLoopDelay({ resolution: 20 })
export function startPerformanceMonitoring(): void {
  delay.enable()
}
export function recordTiming(name: string, milliseconds: number): void {
  if (!Number.isFinite(milliseconds)) return
  const values = samples.get(name) ?? []
  values.push(Math.round(milliseconds * 100) / 100)
  samples.set(name, values.slice(-100))
}
export const timed = (name: string): (() => void) => {
  const start = performance.now()
  return () => recordTiming(name, performance.now() - start)
}
export function performanceSnapshot(): object {
  return {
    eventLoopMs: {
      mean: Number.isFinite(delay.mean) ? Math.round(delay.mean / 1e6) : 0,
      p99: Math.round(delay.percentile(99) / 1e6),
      max: Math.round(delay.max / 1e6),
    },
    operations: Object.fromEntries(
      [...samples].map(([name, values]) => [
        name,
        {
          count: values.length,
          meanMs: Math.round(values.reduce((a, b) => a + b, 0) / values.length),
          maxMs: Math.max(...values),
        },
      ]),
    ),
  }
}
