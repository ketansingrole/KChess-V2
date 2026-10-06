import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createRouter, createWebHistory, type Router } from 'vue-router'
import { defineComponent } from 'vue'
import { usePageSwipeNavigation } from '../../app/composables/usePageSwipeNavigation'

/** The swipe drives a real vue-router: pushes must populate back, go(-1) must move. */
describe('swipe router integration', () => {
  let router: Router
  let time: number
  let wrapper: ReturnType<typeof mount> | null = null

  beforeEach(async () => {
    time = 0
    vi.spyOn(window.performance, 'now').mockImplementation(() => time)
    router = createRouter({
      history: createWebHistory(),
      routes: [
        { path: '/', component: defineComponent({ render: () => null }) },
        { path: '/watch', component: defineComponent({ render: () => null }) },
        { path: '/settings', component: defineComponent({ render: () => null }) },
      ],
    })
    vi.stubGlobal('useRouter', () => router)
    await router.push('/')
    await router.isReady()
    wrapper = mount(defineComponent({ setup: usePageSwipeNavigation, render: () => null }))
  })
  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
    document.body.replaceChildren()
  })

  function wheel(deltaX: number, deltaY = 0) {
    time += 20
    const event = new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaX, deltaY })
    document.body.dispatchEvent(event)
    return event
  }

  it('moves to the previous page after sidebar-style pushes', async () => {
    await router.push('/watch')
    expect(router.options.history.state.back).toBe('/')
    wheel(-150)
    await vi.waitFor(() => expect(router.currentRoute.value.path).toBe('/'))
  })

  it('moves forward again after going back', async () => {
    await router.push('/watch')
    await router.push('/settings')
    time += 301
    wheel(-150)
    await vi.waitFor(() => expect(router.currentRoute.value.path).toBe('/watch'))
    time += 301
    wheel(150)
    await vi.waitFor(() => expect(router.currentRoute.value.path).toBe('/settings'))
  })
})
