/** Bounded line decoding, including heartbeats, with cancellation and a stalled-stream deadline. */
import { logDebug } from './logger.ts'

export async function readLines(
  stream: ReadableStream<Uint8Array>,
  onLine: (line: string) => void,
  options: { signal?: AbortSignal; idleMs?: number; maxLineBytes?: number } = {},
): Promise<void> {
  const reader = stream.getReader()
  const decoder = new TextDecoder()
  let tail = ''
  const max = options.maxLineBytes ?? 2_000_000
  let timer: ReturnType<typeof setTimeout> | undefined
  const cancel = (): void => {
    void reader.cancel().catch((error: unknown) => {
      logDebug('ndjson', 'Stream cancel failed:', error)
    })
  }
  options.signal?.addEventListener('abort', cancel, { once: true })
  try {
    while (true) {
      options.signal?.throwIfAborted()
      const result = await Promise.race([
        reader.read(),
        new Promise<never>((_resolve, reject) => {
          timer = setTimeout(() => {
            reject(new Error('Lichess stream stalled. Reconnecting…'))
            cancel()
          }, options.idleMs ?? 45_000)
        }),
      ])
      clearTimeout(timer)
      options.signal?.throwIfAborted()
      if (result.done) break
      tail += decoder.decode(result.value, { stream: true })
      let start = 0
      let end = -1
      while ((end = tail.indexOf('\n', start)) >= 0) {
        if (end - start > max) throw new Error('Lichess sent an oversized stream record.')
        const line = tail.slice(start, end).trim()
        start = end + 1
        if (line) onLine(line)
      }
      if (start > 0) tail = tail.slice(start)
      if (tail.length > max) throw new Error('Lichess sent an oversized stream record.')
    }
    tail += decoder.decode()
    if (tail.trim()) onLine(tail.trim())
  } finally {
    clearTimeout(timer)
    options.signal?.removeEventListener('abort', cancel)
    await reader.cancel().catch((error: unknown) => {
      logDebug('ndjson', 'Stream cancel failed:', error)
    })
    reader.releaseLock()
  }
}
