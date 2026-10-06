interface PageHistory {
  canGo: (direction: -1 | 1) => boolean
  go: (direction: -1 | 1) => void
}

export interface PageSwipeOptions {
  /** False turns the gesture off entirely: wheel events pass through untouched. */
  isEnabled?: () => boolean
  /** False keeps navigation but hides the edge feedback. */
  showFeedback?: () => boolean
  /** Horizontal travel in pixels that commits the navigation. */
  threshold?: number
}

// Trackpads stream ~30-60 wheel events/s while fingers move, with fractional
// pixel deltas; momentum after liftoff is indistinguishable in the DOM.
const GESTURE_TIMEOUT_MS = 300
/** Trailing window the lock and velocity are read from. */
const WINDOW_MS = 120
/** Windowed horizontal travel that engages the swipe. */
const LOCK_TRAVEL_PX = 10
/** A firm vertical scroll claims the gesture; crosstalk on landing is ~3px. */
const VERTICAL_TRAVEL_PX = 20
const VERTICAL_RATIO = 2
/** Long travel that never resolved horizontal is a diagonal scroll, not a swipe. */
const POISON_TRAVEL_PX = 56
const DEFAULT_THRESHOLD_PX = 100
const READY_RATIO = 0.8
const COMMIT_FLASH_MS = 220
/** A flick commits early: fast, deliberate, still short of the full distance. */
const VELOCITY_COMMIT_PX_S = 2000
const FLICK_TRAVEL_PX = 48
/** A firm swipe after decayed scroll momentum restarts the gesture. */
const SPIKE_MIN_PX = 20
const SPIKE_RATIO = 2.5
const SPIKE_QUIET_PX = 10
const MAX_TRAIL = 12

const BACK_SVG =
  '<svg class="page-swipe-arrow" viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m14 6-6 6 6 6"/></svg>'
const FORWARD_SVG =
  '<svg class="page-swipe-arrow" viewBox="0 0 24 24" width="26" height="26" fill="none" stroke="currentColor" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m10 6 6 6-6 6"/></svg>'

/** Trackpads expose two-finger scrolling as pixel wheel events. */
export function installPageSwipeNavigation(
  document: Document,
  history: PageHistory,
  options: PageSwipeOptions = {},
): () => void {
  const view = document.defaultView!
  const threshold = options.threshold ?? DEFAULT_THRESHOLD_PX
  const isEnabled = options.isEnabled ?? (() => true)
  const showFeedback = options.showFeedback ?? (() => true)
  let lastEvent = -Infinity
  let travelX = 0
  let travelY = 0
  let state: 'pending' | 'horizontal' | 'blocked' | 'finished' = 'pending'
  let cachedDirection: -1 | 1 | null = null
  let canNavigate = false
  let commitTimer: ReturnType<typeof setTimeout> | undefined
  let backEdge: HTMLElement | null = null
  let forwardEdge: HTMLElement | null = null
  /** Recent events; the pointer stays put during a wheel gesture, so the hit test is cached per target. */
  let trail: { t: number; dx: number; dy: number }[] = []
  let reserved: { target: EventTarget | null; result: boolean } | null = null

  function edges(): { back: HTMLElement; forward: HTMLElement } {
    if (!backEdge || !forwardEdge || !backEdge.isConnected || !forwardEdge.isConnected) {
      backEdge?.remove()
      forwardEdge?.remove()
      backEdge = document.createElement('div')
      backEdge.className = 'page-swipe-edge is-back'
      backEdge.setAttribute('aria-hidden', 'true')
      backEdge.innerHTML = BACK_SVG
      forwardEdge = document.createElement('div')
      forwardEdge.className = 'page-swipe-edge is-forward'
      forwardEdge.setAttribute('aria-hidden', 'true')
      forwardEdge.innerHTML = FORWARD_SVG
      document.body.append(backEdge, forwardEdge)
    }
    return { back: backEdge, forward: forwardEdge }
  }

  function activeEdge(direction: -1 | 1): HTMLElement {
    const { back, forward } = edges()
    return direction < 0 ? back : forward
  }

  function hideEdges(): void {
    for (const edge of [backEdge, forwardEdge]) {
      if (!edge) continue
      edge.classList.remove('is-visible', 'is-ready', 'is-blocked', 'is-committed')
      edge.style.removeProperty('--swipe-progress')
    }
  }

  /** The tab slides in with the swipe: hidden off-edge at rest, fully in at commit. */
  function paint(direction: -1 | 1): void {
    if (!showFeedback()) return
    const progress = Math.min(1, Math.abs(travelX) / threshold)
    const { back, forward } = edges()
    const active = direction < 0 ? back : forward
    const idle = direction < 0 ? forward : back
    idle.classList.remove('is-visible', 'is-ready', 'is-blocked')
    idle.style.removeProperty('--swipe-progress')
    active.classList.add('is-visible')
    active.classList.toggle('is-ready', canNavigate && progress >= READY_RATIO)
    active.classList.toggle('is-blocked', !canNavigate)
    active.style.setProperty('--swipe-progress', progress.toFixed(3))
  }

  function resetGesture(): void {
    travelX = 0
    travelY = 0
    state = 'pending'
    cachedDirection = null
    canNavigate = false
    trail = []
    reserved = null
    if (commitTimer !== undefined) {
      clearTimeout(commitTimer)
      commitTimer = undefined
    }
    hideEdges()
  }

  /** Horizontal and vertical travel inside the trailing window. */
  function windowSums(now: number): { x: number; y: number; dt: number; count: number } {
    let x = 0
    let y = 0
    let oldest = now
    let count = 0
    for (let i = trail.length - 1; i >= 0; i--) {
      const event = trail[i]!
      if (event.t <= now - WINDOW_MS) break
      x += event.dx
      y += Math.abs(event.dy)
      oldest = event.t
      count++
    }
    return { x, y, dt: count >= 2 ? Math.max(0, now - oldest) : 0, count }
  }

  /** A firm swipe after decayed momentum is a new gesture, not its tail. */
  function hasSpike(event: WheelEvent): boolean {
    if (trail.length < 2) return false
    const previous = trail.slice(-3)
    const average =
      previous.reduce((sum, entry) => sum + Math.abs(entry.dx) + Math.abs(entry.dy), 0) /
      previous.length
    const instant = Math.abs(event.deltaX) + Math.abs(event.deltaY)
    return average < SPIKE_QUIET_PX && instant > SPIKE_MIN_PX && instant > SPIKE_RATIO * average
  }

  function dialogOpen(): boolean {
    return document.querySelector('[role="dialog"], [role="alertdialog"], dialog[open]') !== null
  }

  function pathReserved(event: WheelEvent): boolean {
    for (const target of event.composedPath()) {
      if (!(target instanceof view.HTMLElement)) continue
      if (target.matches('input, textarea, select') || target.isContentEditable) return true
      if (target.scrollWidth <= target.clientWidth) continue
      if (/^(auto|scroll)$/.test(view.getComputedStyle(target).overflowX)) return true
    }
    return false
  }

  function wheel(event: WheelEvent): void {
    if (!isEnabled()) return
    const now = view.performance.now()
    // Momentum belongs to the same gesture, even after navigation replaces the page.
    if (now - lastEvent > GESTURE_TIMEOUT_MS) resetGesture()
    lastEvent = now
    if (hasSpike(event)) resetGesture()
    trail.push({ t: now, dx: event.deltaX, dy: event.deltaY })
    if (trail.length > MAX_TRAIL) trail.shift()
    if (state === 'blocked') return
    if (
      event.defaultPrevented ||
      event.deltaMode !== 0 ||
      event.ctrlKey ||
      event.metaKey ||
      event.altKey ||
      event.shiftKey ||
      dialogOpen()
    ) {
      state = 'blocked'
      return
    }
    if (!reserved || reserved.target !== event.target)
      reserved = { target: event.target, result: pathReserved(event) }
    if (reserved.result) {
      state = 'blocked'
      return
    }
    if (state === 'finished') {
      event.preventDefault()
      return
    }
    travelX += event.deltaX
    travelY += Math.abs(event.deltaY)
    if (state === 'pending') {
      const window = windowSums(now)
      // A firm vertical scroll claims the gesture for scrolling.
      if (window.y >= VERTICAL_TRAVEL_PX && window.y > Math.abs(window.x) * VERTICAL_RATIO) {
        state = 'blocked'
        return
      }
      if (Math.abs(window.x) >= LOCK_TRAVEL_PX && Math.abs(window.x) > window.y) {
        state = 'horizontal'
      } else if (Math.max(Math.abs(travelX), travelY) >= POISON_TRAVEL_PX) {
        // Long travel that never resolved horizontal is a diagonal scroll, not a swipe.
        state = 'blocked'
        return
      } else return
    }
    const direction: -1 | 1 = travelX < 0 ? -1 : 1
    // Ask once per direction so the tab shows the blocked style early without
    // re-querying history on every wheel tick; reuse the answer to commit.
    if (cachedDirection !== direction) {
      cachedDirection = direction
      canNavigate = history.canGo(direction)
    }
    event.preventDefault()
    paint(direction)
    const window = windowSums(now)
    const flick =
      window.count >= 2 &&
      window.dt > 0 &&
      Math.abs(window.x) / (window.dt / 1000) >= VELOCITY_COMMIT_PX_S
    if (Math.abs(travelX) < (flick ? FLICK_TRAVEL_PX : threshold)) return
    if (Math.abs(travelX) < travelY) return
    state = 'finished'
    if (showFeedback()) {
      if (canNavigate) {
        const edge = activeEdge(direction)
        edge.classList.add('is-committed')
        edge.classList.remove('is-ready')
      }
      if (commitTimer !== undefined) clearTimeout(commitTimer)
      commitTimer = setTimeout(hideEdges, COMMIT_FLASH_MS)
    }
    if (canNavigate) history.go(direction)
  }

  document.addEventListener('wheel', wheel, { passive: false })
  return () => {
    document.removeEventListener('wheel', wheel)
    if (commitTimer !== undefined) clearTimeout(commitTimer)
    backEdge?.remove()
    forwardEdge?.remove()
    backEdge = null
    forwardEdge = null
  }
}
