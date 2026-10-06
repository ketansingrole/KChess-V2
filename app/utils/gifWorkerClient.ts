import type { GifFrame } from './gif'

/** One transferred frame at a time; termination cancels active encoding immediately. */
export class GifWorkerClient {
  private pending?: {
    resolve: (data: Uint8Array | undefined) => void
    reject: (cause: Error) => void
  }
  private failure?: Error
  private abort: () => void
  constructor(
    private worker: Worker,
    private width: number,
    private height: number,
    private signal?: AbortSignal,
  ) {
    this.abort = () => this.fail(new DOMException('Export cancelled.', 'AbortError'))
    worker.onmessage = (event: MessageEvent<{ error?: string; data?: Uint8Array }>) => {
      if (event.data.error) return this.fail(new Error(event.data.error))
      const pending = this.pending
      this.pending = undefined
      pending?.resolve(event.data.data)
    }
    worker.onerror = () => this.fail(new Error('The GIF encoder stopped. Retry the export.'))
    signal?.addEventListener('abort', this.abort, { once: true })
    if (signal?.aborted) this.abort()
  }
  private fail(error: Error): void {
    if (this.failure) return
    this.failure = error
    this.signal?.removeEventListener('abort', this.abort)
    this.worker.onmessage = null
    this.worker.onerror = null
    this.pending?.reject(error)
    this.pending = undefined
    this.worker.terminate()
  }
  private send(frame?: GifFrame): Promise<Uint8Array | undefined> {
    if (this.failure) return Promise.reject(this.failure)
    if (this.pending) return Promise.reject(new Error('A GIF frame is already being encoded.'))
    return new Promise((resolve, reject) => {
      this.pending = { resolve, reject }
      try {
        this.worker.postMessage(
          { width: this.width, height: this.height, frame, finish: !frame },
          frame ? [frame.rgba.buffer] : [],
        )
      } catch (cause) {
        console.warn('[gif-worker] posting frame failed:', cause)
        this.fail(cause instanceof Error ? cause : new Error(String(cause)))
      }
    })
  }
  async add(frame: GifFrame): Promise<void> {
    await this.send(frame)
  }
  async finish(): Promise<Uint8Array> {
    const result = await this.send()
    if (!result) throw new Error('The GIF encoder returned no image.')
    return result
  }
  close(): void {
    this.signal?.removeEventListener('abort', this.abort)
    this.fail(new DOMException('Export cancelled.', 'AbortError'))
  }
}
