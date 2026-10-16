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
  /** The page that follows the fingers a little, as the window does in Safari. */
  surface?: () => HTMLElement | null
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
/** Past the threshold the swipe is armed; falling back below this share of it disarms. */
const DISARM_RATIO = 0.8
/**
 * The DOM never says when the fingers lift. A pause in the stream, or momentum (deltas that
 * only shrink), is the release: an armed swipe commits then, and reversing first cancels it.
 */
const RELEASE_IDLE_MS = 90
const DECAY_EVENTS = 3
const DECAY_RATIO = 0.9
const COMMIT_FLASH_MS = 220
/** How far the page follows the fingers, with growing resistance. */
const SHIFT_MAX_PX = 28
const BLOCKED_SHIFT_MAX_PX = 8
const SHIFT_SETTLE = 'transform 200ms cubic-bezier(0.2, 0.8, 0.2, 1)'
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

/** The installing document's window, which detached test documents may lack. */
function windowOf(document: Document): NonNullable<Document['defaultView']> {
  const view = document.defaultView
  if (!view) throw new Error('No window is available for page swipe navigation.')
  return view
}

/** Trackpads expose two-finger scrolling as pixel wheel events. */
export function installPageSwipeNavigation(
  document: Document,
  history: PageHistory,
  options: PageSwipeOptions = {},
): () => void {
  const view = windowOf(document)
  const threshold = options.threshold ?? DEFAULT_THRESHOLD_PX
  const isEnabled = options.isEnabled ?? (() => true)
  const showFeedback = options.showFeedback ?? (() => true)
  let lastEvent = -Infinity
  let travelX = 0
  let travelY = 0
  let state: 'pending' | 'horizontal' | 'armed' | 'blocked' | 'finished' = 'pending'
  let cachedDirection: -1 | 1 | null = null
  let canNavigate = false
  let commitTimer: ReturnType<typeof setTimeout> | undefined
  let releaseTimer: ReturnType<typeof setTimeout> | undefined
  let shifted: HTMLElement | null = null
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

  /** Moves the page toward the swipe; it eases back when `distance` is 0. */
  function shift(distance: number): void {
    const surface = distance ? (shifted ?? options.surface?.() ?? null) : shifted
    if (!surface) return
    if (distance) {
      shifted = surface
      surface.style.transition = 'none'
      surface.style.transform = `translateX(${distance.toFixed(1)}px)`
      return
    }
    surface.style.transition = SHIFT_SETTLE
    surface.style.transform = ''
    // No transform is left behind: it would re-anchor the page's fixed descendants.
    const settled = surface
    settled.addEventListener(
      'transitionend',
      () => {
        if (!settled.style.transform) settled.style.removeProperty('transition')
      },
      { once: true },
    )
    shifted = null
  }

  /**
   * The tab slides in with the swipe: hidden off-edge at rest, fully in once letting go would
   * navigate. The page follows with resistance, barely at either end of history.
   */
  function paint(direction: -1 | 1): void {
    if (!showFeedback()) return
    const ratio = Math.abs(travelX) / threshold
    const progress = Math.min(1, ratio)
    const { back, forward } = edges()
    const active = direction < 0 ? back : forward
    const idle = direction < 0 ? forward : back
    idle.classList.remove('is-visible', 'is-ready', 'is-blocked')
    idle.style.removeProperty('--swipe-progress')
    active.classList.add('is-visible')
    active.classList.toggle('is-ready', canNavigate && state === 'armed')
    active.classList.toggle('is-blocked', !canNavigate)
    active.style.setProperty('--swipe-progress', progress.toFixed(3))
    const reach = canNavigate ? SHIFT_MAX_PX : BLOCKED_SHIFT_MAX_PX
    // Fingers move right to go back, and the page moves with them.
    shift(-direction * reach * (1 - Math.exp(-ratio)))
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
    clearRelease()
    hideEdges()
    shift(0)
  }

  function clearRelease(): void {
    if (releaseTimer === undefined) return
    clearTimeout(releaseTimer)
    releaseTimer = undefined
  }

  /** The latest deltas toward `direction` only shrink: the fingers have lifted (or are settling). */
  function decaying(direction: -1 | 1): boolean {
    const recent = trail.slice(-DECAY_EVENTS).map((event) => event.dx * direction)
    if (recent.length < DECAY_EVENTS) return false
    let previous = Infinity
    for (const dx of recent) {
      if (dx <= 0 || dx > previous) return false
      previous = dx
    }
    return previous <= (recent[0] ?? 0) * DECAY_RATIO
  }

  /** Navigates an armed swipe, or lets a disarmed one settle back. */
  function release(direction: -1 | 1): void {
    clearRelease()
    if (state !== 'armed') {
      hideEdges()
      shift(0)
      return
    }
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
    shift(0)
    if (canNavigate) history.go(direction)
  }

  /** Horizontal and vertical travel inside the trailing window. */
  function windowSums(now: number): { x: number; y: number; dt: number; count: number } {
    let x = 0
    let y = 0
    let oldest = now
    let count = 0
    for (let i = trail.length - 1; i >= 0; i--) {
      const event = trail[i]
      if (!event || event.t <= now - WINDOW_MS) break
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
      // Reversing past the start is a new swipe the other way, not yet armed.
      if (state === 'armed') state = 'horizontal'
    }
    event.preventDefault()
    const window = windowSums(now)
    const flick =
      window.count >= 2 &&
      window.dt > 0 &&
      Math.abs(window.x) / (window.dt / 1000) >= VELOCITY_COMMIT_PX_S
    const distance = Math.abs(travelX)
    const straight = distance >= travelY
    if (state === 'horizontal') {
      if (canNavigate && straight && distance >= (flick ? FLICK_TRAVEL_PX : threshold))
        state = 'armed'
    } else if (state === 'armed' && (distance < threshold * DISARM_RATIO || !straight)) {
      // Sliding back cancels, as it does in Safari.
      if (!(flick && distance >= FLICK_TRAVEL_PX)) state = 'horizontal'
    }
    paint(direction)
    clearRelease()
    if (state === 'armed' && decaying(direction)) {
      release(direction)
      return
    }
    releaseTimer = setTimeout(() => {
      releaseTimer = undefined
      release(direction)
    }, RELEASE_IDLE_MS)
  }

  document.addEventListener('wheel', wheel, { passive: false })
  return () => {
    document.removeEventListener('wheel', wheel)
    if (commitTimer !== undefined) clearTimeout(commitTimer)
    clearRelease()
    shift(0)
    backEdge?.remove()
    forwardEdge?.remove()
    backEdge = null
    forwardEdge = null
  }
}
