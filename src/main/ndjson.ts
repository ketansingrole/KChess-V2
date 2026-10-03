/** Bounded line decoding, including heartbeats, with cancellation and a stalled-stream deadline. */
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
    void reader.cancel().catch(() => {})
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
      let end: number
      while ((end = tail.indexOf('\n')) >= 0) {
        if (end > max) throw new Error('Lichess sent an oversized stream record.')
        const line = tail.slice(0, end).trim()
        tail = tail.slice(end + 1)
        if (line) onLine(line)
      }
      if (tail.length > max) throw new Error('Lichess sent an oversized stream record.')
    }
    tail += decoder.decode()
    if (tail.trim()) onLine(tail.trim())
  } finally {
    clearTimeout(timer)
    options.signal?.removeEventListener('abort', cancel)
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}
