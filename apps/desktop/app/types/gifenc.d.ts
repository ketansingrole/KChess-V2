declare module 'gifenc' {
  /** A color table: at most 256 `[red, green, blue]` byte triples. */
  export type GifencPalette = number[][]
  /** Reduce RGBA pixels to a color table of at most `maxColors` entries. */
  export function quantize(
    rgba: Uint8Array | Uint8ClampedArray,
    maxColors: number,
    options?: Record<string, unknown>,
  ): GifencPalette
  /** Map each RGBA pixel to its nearest palette index. */
  export function applyPalette(
    rgba: Uint8Array | Uint8ClampedArray,
    palette: GifencPalette,
    format?: string,
  ): Uint8Array
  export interface GifEncoderFrameOptions {
    palette?: GifencPalette | null
    /** Frame delay in milliseconds. */
    delay?: number
    /** -1 once, 0 forever, otherwise a repeat count. */
    repeat?: number
    transparent?: boolean
    transparentIndex?: number
    first?: boolean
    colorDepth?: number
    dispose?: number
  }
  export interface GifEncoder {
    writeFrame(
      index: Uint8Array,
      width: number,
      height: number,
      options?: GifEncoderFrameOptions,
    ): void
    finish(): void
    bytes(): Uint8Array
  }
  export function GIFEncoder(options?: { auto?: boolean; initialCapacity?: number }): GifEncoder
}
