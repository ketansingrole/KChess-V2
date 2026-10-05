import { StreamingGifEncoder, type GifFrame } from './gif'

let encoder: StreamingGifEncoder | undefined
self.addEventListener(
  'message',
  (event: MessageEvent<{ width: number; height: number; frame?: GifFrame; finish?: boolean }>) => {
    try {
      const { width, height, frame, finish } = event.data
      encoder ??= new StreamingGifEncoder(width, height)
      if (finish) {
        const data = encoder.finish()
        self.postMessage({ data }, { transfer: [data.buffer] })
        encoder = undefined
      } else if (frame) {
        encoder.add(frame)
        self.postMessage({ ready: true })
      }
    } catch (cause) {
      self.postMessage({ error: cause instanceof Error ? cause.message : String(cause) })
    }
  },
)
