import { createInterface } from 'node:readline'
import { Readable } from 'node:stream'
import type { ReadableStream as NodeReadableStream } from 'node:stream/web'

/** Read a streaming response line by line (Lichess uses NDJSON for streams; blank lines are keep-alives). */
export async function readLines(
  stream: ReadableStream<Uint8Array>,
  onLine: (line: string) => void,
): Promise<void> {
  const lines = createInterface({
    input: Readable.fromWeb(stream as NodeReadableStream<Uint8Array>),
    crlfDelay: Infinity,
  })
  for await (const raw of lines) {
    const line = raw.trim()
    if (line) onLine(line)
  }
}
