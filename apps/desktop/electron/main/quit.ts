import { randomUUID } from 'node:crypto'
import { logDebug, logWarn } from '@kchess/native/logger'

interface QuitWindow {
  send(token: string): void
  onClosed(callback: () => void): () => void
  /** True when the renderer process is gone (crashed): there is nothing left to flush. */
  gone(): boolean
}

/** The user's answer after a flush failed or went unanswered. */
export type QuitRetryChoice = 'retry' | 'discard' | 'cancel'
/** What the renderer is being replaced for: closing it or reloading it. */
export type QuitPurpose = 'quit' | 'reload'

/** A failed or unanswered flush never authorizes closing the renderer or its core. */
export class QuitPreparation {
  private pending?: { token: string; resolve: (saved: boolean) => void }
  private preparing?: Promise<boolean>

  constructor(
    private options: {
      window: () => QuitWindow | undefined
      retry: (purpose: QuitPurpose) => Promise<QuitRetryChoice>
      timeoutMs?: number
    },
  ) {}

  complete(token: string, saved = true): void {
    if (!this.pending || token !== this.pending.token) throw new Error('No matching quit request.')
    this.pending.resolve(saved)
  }

  prepare(purpose: QuitPurpose = 'quit'): Promise<boolean> {
    this.preparing ??= this.perform(purpose)
      .catch((cause) => {
        logWarn('shutdown', 'Quit preparation failed.', cause)
        return false
      })
      .finally(() => {
        this.preparing = undefined
      })
    return this.preparing
  }

  private async perform(purpose: QuitPurpose): Promise<boolean> {
    while (true) {
      const owner = this.options.window()
      // No window, or a crashed renderer: nothing remains to flush, so quitting is safe.
      if (!owner || owner.gone()) return true
      const token = randomUUID()
      let resolve!: (saved: boolean) => void
      const ready = new Promise<boolean>((done) => {
        resolve = done
      })
      this.pending = { token, resolve }
      const off = owner.onClosed(() => resolve(false))
      const timer = setTimeout(() => {
        logWarn('shutdown', 'Renderer flush timed out.')
        resolve(false)
      }, this.options.timeoutMs ?? 3000)
      let saved: boolean
      try {
        owner.send(token)
        saved = await ready
      } finally {
        clearTimeout(timer)
        off()
        this.pending = undefined
      }
      if (saved) return true
      const choice = await this.options.retry(purpose)
      if (choice === 'discard') {
        logDebug('shutdown', `Continuing (${purpose}) without saving after a failed flush.`)
        return true
      }
      if (choice === 'cancel') return false
    }
  }
}
