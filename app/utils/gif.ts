/**
 * A small animated-GIF encoder (GIF89a, LZW, one global palette), so exporting a game needs no
 * dependency. Frames are RGBA pixel arrays of the same size.
 */

export interface GifFrame {
  /** RGBA, width × height × 4 bytes. */
  rgba: Uint8ClampedArray
  /** How long the frame shows, in hundredths of a second. */
  delay: number
}

/** 15-bit colour key (5 bits per channel). */
const key = (r: number, g: number, b: number): number =>
  ((r >> 3) << 10) | ((g >> 3) << 5) | (b >> 3)

/**
 * A palette of the most common colours across the frames (boards have few), with every other
 * colour mapped to its nearest entry. Returns the palette and an index lookup by colour key.
 */
export function buildPalette(frames: readonly GifFrame[]): {
  palette: Uint8Array
  lookup: Uint8Array
} {
  const counts = new Uint32Array(32768)
  for (const frame of frames) {
    const data = frame.rgba
    // Every pixel of the first frame, then a sample of the rest: later frames differ little.
    const step = frame === frames[0] ? 4 : 16
    for (let i = 0; i < data.length; i += step) counts[key(data[i]!, data[i + 1]!, data[i + 2]!)]!++
  }
  const used: number[] = []
  for (let k = 0; k < 32768; k++) if (counts[k]) used.push(k)
  used.sort((a, b) => counts[b]! - counts[a]!)
  const chosen = used.slice(0, 256)
  const palette = new Uint8Array(256 * 3)
  chosen.forEach((k, i) => {
    palette[i * 3] = ((k >> 10) & 31) * 8 + 4
    palette[i * 3 + 1] = ((k >> 5) & 31) * 8 + 4
    palette[i * 3 + 2] = (k & 31) * 8 + 4
  })
  const lookup = new Uint8Array(32768)
  const exact = new Map(chosen.map((k, i) => [k, i]))
  for (const k of used) {
    const hit = exact.get(k)
    if (hit !== undefined) {
      lookup[k] = hit
      continue
    }
    const r = (k >> 10) & 31,
      g = (k >> 5) & 31,
      b = k & 31
    let best = 0,
      bestDistance = Infinity
    for (let i = 0; i < chosen.length; i++) {
      const c = chosen[i]!
      const dr = ((c >> 10) & 31) - r,
        dg = ((c >> 5) & 31) - g,
        db = (c & 31) - b
      const distance = 3 * dr * dr + 4 * dg * dg + 2 * db * db
      if (distance < bestDistance) {
        bestDistance = distance
        best = i
      }
    }
    lookup[k] = best
  }
  return { palette, lookup }
}

class ByteWriter {
  private chunks: Uint8Array[] = []
  private buffer = new Uint8Array(65536)
  private length = 0
  byte(value: number): void {
    if (this.length === this.buffer.length) this.flush()
    this.buffer[this.length++] = value & 255
  }
  bytes(values: ArrayLike<number>): void {
    for (let i = 0; i < values.length; i++) this.byte(values[i]!)
  }
  short(value: number): void {
    this.byte(value)
    this.byte(value >> 8)
  }
  text(value: string): void {
    for (let i = 0; i < value.length; i++) this.byte(value.charCodeAt(i))
  }
  private flush(): void {
    this.chunks.push(this.buffer.slice(0, this.length))
    this.length = 0
  }
  result(): Uint8Array {
    this.flush()
    const total = this.chunks.reduce((sum, chunk) => sum + chunk.length, 0)
    const out = new Uint8Array(total)
    let at = 0
    for (const chunk of this.chunks) {
      out.set(chunk, at)
      at += chunk.length
    }
    return out
  }
}

/** GIF's variable-width LZW, written as data sub-blocks. */
function lzw(writer: ByteWriter, indices: Uint8Array, minCodeSize = 8): void {
  const clear = 1 << minCodeSize
  const end = clear + 1
  let codeSize = minCodeSize + 1
  let next = end + 1
  let dictionary = new Map<number, number>()
  const block: number[] = []
  let bitBuffer = 0
  let bitCount = 0
  const emit = (code: number): void => {
    bitBuffer |= code << bitCount
    bitCount += codeSize
    while (bitCount >= 8) {
      block.push(bitBuffer & 255)
      bitBuffer >>>= 8
      bitCount -= 8
      if (block.length === 255) {
        writer.byte(255)
        writer.bytes(block)
        block.length = 0
      }
    }
  }
  writer.byte(minCodeSize)
  emit(clear)
  let prefix = indices[0]!
  for (let i = 1; i < indices.length; i++) {
    const value = indices[i]!
    const entry = prefix * 256 + value
    const known = dictionary.get(entry)
    if (known !== undefined) {
      prefix = known
      continue
    }
    emit(prefix)
    if (next < 4096) {
      dictionary.set(entry, next++)
      if (next > 1 << codeSize && codeSize < 12) codeSize++
    } else {
      emit(clear)
      dictionary = new Map()
      codeSize = minCodeSize + 1
      next = end + 1
    }
    prefix = value
  }
  emit(prefix)
  emit(end)
  if (bitCount > 0) block.push(bitBuffer & 255)
  for (let at = 0; at < block.length; at += 255) {
    const part = block.slice(at, at + 255)
    writer.byte(part.length)
    writer.bytes(part)
  }
  writer.byte(0)
}

/** Encode frames of `width` × `height` into an animated GIF that loops forever. */
export function encodeGif(width: number, height: number, frames: readonly GifFrame[]): Uint8Array {
  if (!frames.length) throw new Error('Nothing to export.')
  const { palette, lookup } = buildPalette(frames)
  const writer = new ByteWriter()
  writer.text('GIF89a')
  writer.short(width)
  writer.short(height)
  writer.byte(0xf7) // global colour table, 8 bits per colour, 256 entries
  writer.byte(0)
  writer.byte(0)
  writer.bytes(palette)
  // Loop forever (NETSCAPE2.0).
  writer.bytes([0x21, 0xff, 0x0b])
  writer.text('NETSCAPE2.0')
  writer.bytes([3, 1, 0, 0, 0])
  const indices = new Uint8Array(width * height)
  for (const frame of frames) {
    writer.bytes([0x21, 0xf9, 4, 0])
    writer.short(Math.max(2, Math.round(frame.delay)))
    writer.bytes([0, 0])
    writer.byte(0x2c)
    writer.short(0)
    writer.short(0)
    writer.short(width)
    writer.short(height)
    writer.byte(0)
    const data = frame.rgba
    for (let p = 0, i = 0; p < indices.length; p++, i += 4)
      indices[p] = lookup[key(data[i]!, data[i + 1]!, data[i + 2]!)]!
    lzw(writer, indices)
  }
  writer.byte(0x3b)
  return writer.result()
}
