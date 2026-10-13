import { randomUUID } from 'node:crypto'
import { logWarn } from '@kchess/native/logger'

interface QuitWindow {
  send(token: string): void
  onClosed(callback: () => void): () => void
}

/** A failed or unanswered flush never authorizes closing the renderer or its core. */
export class QuitPreparation {
  private pending?: { token: string; resolve: (saved: boolean) => void }
  private preparing?: Promise<boolean>

  constructor(
    private options: {
      window: () => QuitWindow | undefined
      retry: () => Promise<boolean>
      timeoutMs?: number
    },
  ) {}

  complete(token: string, saved = true): void {
    if (!this.pending || token !== this.pending.token) throw new Error('No matching quit request.')
    this.pending.resolve(saved)
  }

  prepare(): Promise<boolean> {
    this.preparing ??= this.perform()
      .catch((cause) => {
        logWarn('shutdown', 'Quit preparation failed.', cause)
        return false
      })
      .finally(() => {
        this.preparing = undefined
      })
    return this.preparing
  }

  private async perform(): Promise<boolean> {
    while (true) {
      const owner = this.options.window()
      if (!owner) return true
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
      if (!(await this.options.retry())) return false
    }
  }
}
