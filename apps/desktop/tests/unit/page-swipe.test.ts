import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { installPageSwipeNavigation } from '../../app/utils/pageSwipe'
import { usePageSwipeNavigation } from '../../app/composables/usePageSwipeNavigation'
import { useSwipeBack } from '../../app/composables/useSwipeBack'
import {
  activeSwipeBackStep,
  activeSwipeForwardStep,
  registerSwipeBackStep,
} from '../../app/utils/swipeBack'
import { DEFAULT_SETTINGS } from '@kchess/core/contracts/defaultSettings'
import { assertSettings } from '@kchess/core/domain/validate'

describe('trackpad page history', () => {
  let dispose: () => void
  let time: number
  const go = vi.fn()
  const canGo = vi.fn(() => true)

  beforeEach(() => {
    time = 0
    vi.spyOn(window.performance, 'now').mockImplementation(() => time)
    canGo.mockReturnValue(true)
    dispose = installPageSwipeNavigation(document, { go, canGo })
  })
  afterEach(() => {
    dispose()
    document.body.replaceChildren()
  })

  function wheel(
    deltaX: number,
    deltaY = 0,
    target: EventTarget = document.body,
    init: WheelEventInit = {},
  ) {
    time += 20
    const event = new WheelEvent('wheel', {
      bubbles: true,
      cancelable: true,
      deltaX,
      deltaY,
      ...init,
    })
    // happy-dom's WheelEvent constructor does not initialize MouseEvent modifiers.
    for (const key of ['ctrlKey', 'metaKey', 'altKey', 'shiftKey'] as const) {
      if (init[key]) Object.defineProperty(event, key, { value: true })
    }
    target.dispatchEvent(event)
    return event
  }

  it('accumulates a swipe and moves once, ignoring its momentum on the next page', () => {
    wheel(-40)
    expect(go).not.toHaveBeenCalled()
    wheel(-70)
    expect(go).toHaveBeenCalledExactlyOnceWith(-1)
    document.body.replaceChildren(document.createElement('main'))
    for (let i = 0; i < 40; i++) wheel(-20)
    expect(go).toHaveBeenCalledTimes(1)
    time += 301
    wheel(110)
    expect(go).toHaveBeenLastCalledWith(1)
    expect(go).toHaveBeenCalledTimes(2)
  })

  it('swallows a discrete repeat like Playwright momentum, then navigates again after a pause', () => {
    wheel(-120)
    expect(go).toHaveBeenCalledExactlyOnceWith(-1)
    wheel(-120)
    expect(go).toHaveBeenCalledTimes(1)
    time += 350
    wheel(120)
    expect(go).toHaveBeenLastCalledWith(1)
    expect(go).toHaveBeenCalledTimes(2)
  })

  it('does not leave the app or navigate beyond either end of history', () => {
    canGo.mockReturnValue(false)
    wheel(-120)
    time += 301
    wheel(120)
    expect(canGo.mock.calls).toEqual([[-1], [1]])
    expect(go).not.toHaveBeenCalled()
  })

  it('locks vertical and diagonal scroll gestures even if they later drift sideways', () => {
    expect(wheel(5, 30).defaultPrevented).toBe(false)
    wheel(140, 1)
    time += 301
    expect(wheel(90, 90).defaultPrevented).toBe(false)
    wheel(140)
    expect(go).not.toHaveBeenCalled()
  })

  it('requires deliberate travel and lets direction reversals cancel it', () => {
    wheel(-50)
    wheel(50)
    wheel(60)
    expect(go).not.toHaveBeenCalled()
    wheel(50)
    expect(go).toHaveBeenCalledExactlyOnceWith(1)
  })

  it.each([
    { ctrlKey: true },
    { metaKey: true },
    { shiftKey: true },
    { altKey: true },
    { deltaMode: 1 },
    { deltaMode: 2 },
  ])('preserves zoom, modifier gestures and non-pixel wheel events: %j', (init) => {
    expect(wheel(150, 0, document.body, init).defaultPrevented).toBe(false)
    expect(go).not.toHaveBeenCalled()
  })

  it('preserves wheel handling already claimed by a component', () => {
    const target = document.createElement('div')
    document.body.append(target)
    target.addEventListener('wheel', (event) => event.preventDefault())
    wheel(150, 0, target)
    expect(go).not.toHaveBeenCalled()
  })

  it('reserves horizontal scrollers, including at their edge and after pointer movement', () => {
    const scroller = document.createElement('div')
    scroller.style.overflowX = 'auto'
    Object.defineProperties(scroller, { scrollWidth: { value: 400 }, clientWidth: { value: 100 } })
    const child = document.createElement('span')
    scroller.append(child)
    document.body.append(scroller)
    expect(wheel(-120, 0, child).defaultPrevented).toBe(false)
    wheel(-120)
    expect(go).not.toHaveBeenCalled()
  })

  it.each(['input', 'textarea', 'select'])('preserves editing in %s', (tag) => {
    const target = document.createElement(tag)
    document.body.append(target)
    expect(wheel(150, 0, target).defaultPrevented).toBe(false)
    expect(go).not.toHaveBeenCalled()
  })

  it('does not navigate while a dialog is open, even with the pointer outside it', () => {
    const dialog = document.createElement('div')
    dialog.setAttribute('role', 'dialog')
    document.body.append(dialog)
    expect(wheel(150).defaultPrevented).toBe(false)
    expect(go).not.toHaveBeenCalled()
  })

  it('removes its listener on disposal', () => {
    dispose()
    wheel(150)
    expect(go).not.toHaveBeenCalled()
  })

  it('shows edge feedback that tracks the swipe and flashes on commit', () => {
    wheel(-40)
    const back = document.querySelector<HTMLElement>('.page-swipe-edge.is-back')
    const forward = document.querySelector<HTMLElement>('.page-swipe-edge.is-forward')
    expect(back?.classList.contains('is-visible')).toBe(true)
    expect(back?.classList.contains('is-blocked')).toBe(false)
    expect(back?.style.getPropertyValue('--swipe-progress')).toBe('0.400')
    expect(forward?.classList.contains('is-visible')).toBe(false)
    expect(go).not.toHaveBeenCalled()
    wheel(-70)
    expect(go).toHaveBeenCalledExactlyOnceWith(-1)
    expect(back?.classList.contains('is-committed')).toBe(true)
  })

  it('shows blocked feedback without navigating at either end of history', () => {
    canGo.mockReturnValue(false)
    const event = wheel(-120)
    expect(event.defaultPrevented).toBe(true)
    expect(go).not.toHaveBeenCalled()
    const back = document.querySelector('.page-swipe-edge.is-back')
    expect(back?.classList.contains('is-visible')).toBe(true)
    expect(back?.classList.contains('is-blocked')).toBe(true)
    expect(back?.classList.contains('is-committed')).toBe(false)
  })

  it('leaves wheel events alone when the gesture is turned off', () => {
    dispose()
    dispose = installPageSwipeNavigation(document, { go, canGo }, { isEnabled: () => false })
    expect(wheel(150).defaultPrevented).toBe(false)
    expect(go).not.toHaveBeenCalled()
    expect(document.querySelector('.page-swipe-edge')).toBeNull()
  })

  it('navigates without feedback when the indicator is off', () => {
    dispose()
    dispose = installPageSwipeNavigation(document, { go, canGo }, { showFeedback: () => false })
    wheel(-40)
    expect(document.querySelector('.page-swipe-edge')).toBeNull()
    wheel(-70)
    expect(go).toHaveBeenCalledExactlyOnceWith(-1)
  })

  it('removes its edge nodes on disposal', () => {
    wheel(-40)
    expect(document.querySelectorAll('.page-swipe-edge')).toHaveLength(2)
    dispose()
    dispose = () => {}
    expect(document.querySelector('.page-swipe-edge')).toBeNull()
  })

  it('uses Vue Router history and detaches when the app unmounts', () => {
    dispose()
    const state = { back: null as string | null, forward: null as string | null }
    vi.stubGlobal('useRouter', () => ({ options: { history: { state } }, go }))
    const wrapper = mount(defineComponent({ setup: usePageSwipeNavigation, render: () => null }))
    wheel(-150)
    expect(go).not.toHaveBeenCalled()
    state.back = '/analysis'
    time += 301
    wheel(-150)
    expect(go).toHaveBeenCalledExactlyOnceWith(-1)
    state.forward = '/settings'
    time += 301
    wheel(150)
    expect(go).toHaveBeenLastCalledWith(1)
    wrapper.unmount()
    time += 301
    wheel(-150)
    expect(go).toHaveBeenCalledTimes(2)
  })

  it('steps out of the page before leaving it', () => {
    dispose()
    const localBack = vi.fn()
    let canLocal = true
    const state = { back: null as string | null, forward: null as string | null }
    const routerGo = vi.fn()
    vi.stubGlobal('useRouter', () => ({ options: { history: { state } }, go: routerGo }))
    const wrapper = mount(
      defineComponent({
        setup() {
          useSwipeBack({ canBack: () => canLocal, back: localBack })
          usePageSwipeNavigation()
          return () => null
        },
      }),
    )
    wheel(-150)
    expect(localBack).toHaveBeenCalledTimes(1)
    expect(routerGo).not.toHaveBeenCalled()
    const back = document.querySelector('.page-swipe-edge.is-back')
    expect(back?.classList.contains('is-visible')).toBe(true)
    expect(back?.classList.contains('is-blocked')).toBe(false)
    // Inner levels exhausted: the same swipe falls back to router history.
    canLocal = false
    state.back = '/analysis'
    time += 301
    wheel(-150)
    expect(localBack).toHaveBeenCalledTimes(1)
    expect(routerGo).toHaveBeenCalledExactlyOnceWith(-1)
    wrapper.unmount()
  })

  it('redoes the last swipe-back while nothing changed since', () => {
    dispose()
    const localBack = vi.fn()
    const localForward = vi.fn()
    let canLocalBack = true
    let canLocalForward = false
    const state = { back: null as string | null, forward: null as string | null }
    const routerGo = vi.fn()
    vi.stubGlobal('useRouter', () => ({ options: { history: { state } }, go: routerGo }))
    const wrapper = mount(
      defineComponent({
        setup() {
          useSwipeBack({
            canBack: () => canLocalBack,
            back: () => {
              localBack()
              canLocalBack = false
              canLocalForward = true
            },
            canForward: () => canLocalForward,
            forward: () => {
              localForward()
              canLocalForward = false
            },
          })
          usePageSwipeNavigation()
          return () => null
        },
      }),
    )
    wheel(-150)
    expect(localBack).toHaveBeenCalledTimes(1)
    expect(routerGo).not.toHaveBeenCalled()
    time += 301
    wheel(150)
    expect(localForward).toHaveBeenCalledTimes(1)
    expect(routerGo).not.toHaveBeenCalled()
    const forward = document.querySelector('.page-swipe-edge.is-forward')
    expect(forward?.classList.contains('is-committed')).toBe(true)
    // Redo consumed: the next swipe falls back to router history.
    state.forward = '/settings'
    time += 301
    wheel(150)
    expect(localForward).toHaveBeenCalledTimes(1)
    expect(routerGo).toHaveBeenCalledExactlyOnceWith(1)
    wrapper.unmount()
  })

  /** Realistic trackpad streams: finger landing, flicks, jitter, drift, momentum. */
  function trackpad(stream: [number, number, number][]): void {
    for (const [dx, dy, dt] of stream) {
      time += dt
      const event = new WheelEvent('wheel', {
        bubbles: true,
        cancelable: true,
        deltaX: dx,
        deltaY: dy,
      })
      document.body.dispatchEvent(event)
    }
  }
  const rep = (dx: number, dy: number, dt: number, n: number): [number, number, number][] =>
    Array.from({ length: n }, () => [dx, dy, dt] as [number, number, number])

  it('commits a swipe that lands diagonally then straightens', () => {
    trackpad([[-8, 12, 20], [-8, 12, 20], ...rep(-15, 1, 20, 8)])
    expect(go).toHaveBeenCalledExactlyOnceWith(-1)
  })

  it('commits fast flicks below the full distance', () => {
    trackpad([...rep(-35, 2, 16, 2), ...rep(-20, 2, 16, 2)])
    expect(go).toHaveBeenCalledExactlyOnceWith(-1)
  })

  it('never commits shaky back-and-forth jitter', () => {
    const alternating: [number, number, number][] = []
    for (let i = 0; i < 6; i++) alternating.push([30, 2, 20], [-30, 2, 20])
    trackpad(alternating)
    expect(go).not.toHaveBeenCalled()
  })

  it('ignores a sustained diagonal drift', () => {
    trackpad(rep(10, 10, 30, 8))
    expect(go).not.toHaveBeenCalled()
  })

  it('starts a new gesture on a firm swipe after scroll momentum', () => {
    trackpad([...rep(0, 42, 20, 3), ...rep(0, 6, 20, 3), ...rep(-26, 3, 20, 5)])
    expect(go).toHaveBeenCalledExactlyOnceWith(-1)
  })

  it('commits a steady swipe with vertical wobble', () => {
    trackpad(rep(-12, 4, 25, 9))
    expect(go).toHaveBeenCalledExactlyOnceWith(-1)
  })
})

describe('swipe-back steps', () => {
  it('keeps the latest registration and ignores stale unregisters', () => {
    const backA = vi.fn()
    const backB = vi.fn()
    const unregisterA = registerSwipeBackStep({ canBack: () => true, back: backA })
    const unregisterB = registerSwipeBackStep({ canBack: () => true, back: backB })
    activeSwipeBackStep()?.back()
    expect(backB).toHaveBeenCalledTimes(1)
    expect(backA).not.toHaveBeenCalled()
    unregisterA()
    activeSwipeBackStep()?.back()
    expect(backB).toHaveBeenCalledTimes(2)
    unregisterB()
    expect(activeSwipeBackStep()).toBeNull()
  })

  it('skips steps with nothing to unwind', () => {
    const unregister = registerSwipeBackStep({ canBack: () => false, back: () => {} })
    expect(activeSwipeBackStep()).toBeNull()
    unregister()
    expect(activeSwipeBackStep()).toBeNull()
  })

  it('offers redo only while the undone state is intact', () => {
    const forward = vi.fn()
    const unregister = registerSwipeBackStep({
      canBack: () => false,
      back: () => {},
      canForward: () => true,
      forward,
    })
    expect(activeSwipeBackStep()).toBeNull()
    activeSwipeForwardStep()?.forward?.()
    expect(forward).toHaveBeenCalledTimes(1)
    unregister()
    expect(activeSwipeForwardStep()).toBeNull()
  })

  it('has no forward step without redo handlers', () => {
    const unregister = registerSwipeBackStep({ canBack: () => true, back: () => {} })
    expect(activeSwipeForwardStep()).toBeNull()
    unregister()
  })
})

describe('swipe settings', () => {
  it('leave the gesture and its feedback on by default', () => {
    expect(DEFAULT_SETTINGS.swipeNavigation).toBe(true)
    expect(DEFAULT_SETTINGS.swipeIndicator).toBe(true)
  })

  it('accepts both switches and rejects non-booleans', () => {
    expect(assertSettings({ ...DEFAULT_SETTINGS, swipeNavigation: false }).swipeNavigation).toBe(
      false,
    )
    expect(assertSettings({ ...DEFAULT_SETTINGS }).swipeIndicator).toBe(true)
    expect(() => assertSettings({ ...DEFAULT_SETTINGS, swipeIndicator: 'yes' })).toThrow()
  })
})
