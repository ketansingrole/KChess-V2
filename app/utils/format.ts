/** Human-readable size: 0 B, 512 B, 3.4 KB, 12 MB … */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB']
  const exponent = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)))
  const value = bytes / 1024 ** exponent
  return `${exponent === 0 || value >= 100 ? Math.round(value) : value.toFixed(1)} ${units[exponent]}`
}

export function formatCount(count: number): string {
  return count.toLocaleString('en-US')
}

/** "2 min ago", "3 days ago"; falls back to a date for anything older than a month. */
export function timeAgo(timestamp: number | undefined, now = Date.now()): string {
  if (!timestamp) return 'never'
  const seconds = Math.max(0, Math.round((now - timestamp) / 1000))
  if (seconds < 45) return 'just now'
  const minutes = Math.round(seconds / 60)
  if (minutes < 60) return `${minutes} min ago`
  const hours = Math.round(minutes / 60)
  if (hours < 24) return `${hours} h ago`
  const days = Math.round(hours / 24)
  if (days < 31) return `${days} day${days === 1 ? '' : 's'} ago`
  return new Date(timestamp).toLocaleDateString()
}

/** Connection quality from a round trip, using the same bands Lichess uses for its signal bars. */
export function signalFromLatency(ms: number): 1 | 2 | 3 | 4 {
  if (ms < 150) return 4
  if (ms < 300) return 3
  if (ms < 500) return 2
  return 1
}
