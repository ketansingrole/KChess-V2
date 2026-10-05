import { describe, expect, it } from 'vitest'
import { encodeGif } from '../../app/utils/gif'

/** A minimal GIF decoder (global palette, full frames) to check the encoder round-trips. */
function decode(bytes: Uint8Array): {
  width: number
  height: number
  frames: number[][]
  delays: number[]
} {
  let at = 6
  const short = (): number => bytes[at++]! | (bytes[at++]! << 8)
  const width = short(),
    height = short()
  const flags = bytes[at++]!
  at += 2
  const palette = bytes.slice(at, at + 3 * (1 << ((flags & 7) + 1)))
  at += palette.length
  const frames: number[][] = []
  const delays: number[] = []
  while (bytes[at] !== 0x3b) {
    const block = bytes[at++]
    if (block === 0x21) {
      const label = bytes[at++]
      // Sub-blocks until a zero-length one; the graphic control block holds the delay.
      if (label === 0xf9) delays.push(bytes[at + 2]! | (bytes[at + 3]! << 8))
      while (bytes[at]) at += bytes[at]! + 1
      at++
      continue
    }
    expect(block).toBe(0x2c)
    at += 9
    const minCode = bytes[at++]!
    const data: number[] = []
    while (bytes[at]) {
      const size = bytes[at++]!
      for (let i = 0; i < size; i++) data.push(bytes[at++]!)
    }
    at++
    // LZW decode.
    const clear = 1 << minCode
    let size = minCode + 1
    let dict: number[][] = []
    const reset = (): void => {
      dict = Array.from({ length: clear + 2 }, (_, i) => [i])
      size = minCode + 1
    }
    reset()
    let bit = 0
    const read = (): number => {
      let code = 0
      for (let i = 0; i < size; i++, bit++) code |= ((data[bit >> 3]! >> (bit & 7)) & 1) << i
      return code
    }
    const out: number[] = []
    let previous: number[] | null = null
    for (;;) {
      const code = read()
      if (code === clear) {
        reset()
        previous = null
        continue
      }
      if (code === clear + 1) break
      const entry: number[] = code < dict.length ? dict[code]! : [...previous!, previous![0]!]
      out.push(...entry)
      if (previous) dict.push([...previous, entry[0]!])
      if (dict.length === 1 << size && size < 12) size++
      previous = entry
    }
    frames.push(out.map((index) => palette[index * 3]!))
  }
  return { width, height, frames, delays }
}

describe('GIF export', () => {
  it('round-trips frames through LZW, including table resets', () => {
    const width = 120,
      height = 90
    let seed = 3
    const random = (): number => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff
    const shades = Array.from({ length: 64 }, (_, i) => i * 4)
    const frames = [0, 1].map(() => {
      const rgba = new Uint8ClampedArray(width * height * 4)
      for (let p = 0; p < width * height; p++) {
        const shade = shades[Math.floor(random() * shades.length)]!
        rgba.set([shade, shade, shade, 255], p * 4)
      }
      return { rgba, delay: 40 }
    })
    const decoded = decode(encodeGif(width, height, frames))
    expect([decoded.width, decoded.height]).toEqual([width, height])
    expect(decoded.delays).toEqual([40, 40])
    for (const [index, frame] of decoded.frames.entries()) {
      expect(frame).toHaveLength(width * height)
      frame.forEach((red, p) =>
        expect(Math.abs(red - frames[index]!.rgba[p * 4]!)).toBeLessThanOrEqual(4),
      )
    }
  })

  it('refuses to make an empty GIF', () => {
    expect(() => encodeGif(8, 8, [])).toThrow()
  })
})
