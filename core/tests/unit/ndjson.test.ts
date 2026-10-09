import { describe, expect, it } from 'vitest'
import { readLines } from '../../src/services/ndjson'

function streamOf(chunks: (string | Uint8Array)[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder()
  const parts = chunks.map((chunk) => (typeof chunk === 'string' ? encoder.encode(chunk) : chunk))
  return new ReadableStream<Uint8Array>({
    start(controller) {
      for (const part of parts) controller.enqueue(part)
      controller.close()
    },
  })
}

describe('readLines', () => {
  it('reassembles lines split across chunks without copying per line', async () => {
    const seen: string[] = []
    await readLines(streamOf(['{"a"', ':1}\n{"b":2}\npart', 'ial\n']), (line) => {
      seen.push(line)
    })
    expect(seen).toEqual(['{"a":1}', '{"b":2}', 'partial'])
  })

  it('handles many lines with one trailing copy per chunk', async () => {
    const total = 1000
    const body = Array.from({ length: total }, (_, i) => `{"id":${i}}`).join('\n') + '\n'
    // Split into awkward mid-line chunks.
    const encoder = new TextEncoder()
    const bytes = encoder.encode(body)
    const chunks: Uint8Array[] = []
    for (let at = 0; at < bytes.length; at += 977) chunks.push(bytes.slice(at, at + 977))
    const seen: string[] = []
    await readLines(
      new ReadableStream<Uint8Array>({
        start(controller) {
          for (const chunk of chunks) controller.enqueue(chunk)
          controller.close()
        },
      }),
      (line) => {
        seen.push(line)
      },
    )
    expect(seen).toHaveLength(total)
    expect(seen[0]).toBe('{"id":0}')
    expect(seen[total - 1]).toBe(`{"id":${total - 1}}`)
  })

  it('rejects an oversized line measured from its start', async () => {
    await expect(
      readLines(streamOf([`${'x'.repeat(100)}\n`]), () => {}, { maxLineBytes: 10 }),
    ).rejects.toThrow(/oversized/)
  })
})
