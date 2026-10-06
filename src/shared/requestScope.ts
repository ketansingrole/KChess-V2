/** A request owns a generation and a signal. Superseding it cancels work and rejects late replies. */
export class RequestScope {
  private generation = 0
  private controller = new AbortController()

  capture(): { signal: AbortSignal; current: () => boolean } {
    const generation = this.generation
    const signal = this.controller.signal
    return { signal, current: () => generation === this.generation && !signal.aborted }
  }

  next(): ReturnType<RequestScope['capture']> {
    this.invalidate()
    return this.capture()
  }

  invalidate(): void {
    this.generation++
    const previous = this.controller
    this.controller = new AbortController()
    previous.abort()
  }
}

/** One subscription set per owner. Detach attempts every cleanup even when one throws. */
export class SubscriptionScope {
  private cleanups: (() => void)[] = []

  attach(subscribe: (() => () => void)[]): void {
    if (this.cleanups.length) return
    try {
      for (const start of subscribe) this.cleanups.push(start())
    } catch (cause) {
      this.detach()
      throw cause
    }
  }

  detach(): void {
    const cleanups = this.cleanups
    this.cleanups = []
    let failure: unknown
    for (const cleanup of cleanups) {
      try {
        cleanup()
      } catch (cause) {
        console.warn('[requestScope] Subscription cleanup failed', cause)
        failure ??= cause
      }
    }
    if (failure) throw failure
  }
}
