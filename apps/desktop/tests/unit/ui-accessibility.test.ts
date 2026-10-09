import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import AppCommandPalette from '../../app/components/AppCommandPalette.vue'
import AppToaster from '../../app/components/AppToaster.vue'
import {
  exposeToastFocusGuards,
  labelSearchResults,
  observeUiAccessibility,
} from '../../app/utils/uiAccessibility'

const stops: (() => void)[] = []
afterEach(() => {
  stops.splice(0).forEach((stop) => stop())
  document.body.replaceChildren()
})
const flush = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('component-owned accessibility repairs', () => {
  it('ignores analysis and navigation mutations while repairing late palette content', async () => {
    const root = document.createElement('div')
    const page = document.createElement('main')
    document.body.append(root, page)
    const repair = vi.fn(labelSearchResults)
    stops.push(observeUiAccessibility(root, repair))
    repair.mockClear()
    for (let i = 0; i < 10; i++) {
      page.replaceChildren(document.createElement('span'))
      await flush()
    }
    expect(repair).not.toHaveBeenCalled()

    root.innerHTML = '<div class="kchess-search-palette"><div role="listbox"></div></div>'
    await flush()
    expect(root.querySelector('[role="listbox"]')?.getAttribute('aria-label')).toBe(
      'Search results',
    )
    // Filtering/reopening may replace the content. The new listbox must also be named.
    root.querySelector('.kchess-search-palette')!.innerHTML = '<div role="listbox"></div>'
    await flush()
    expect(root.querySelector('[role="listbox"]')?.getAttribute('aria-label')).toBe(
      'Search results',
    )
    expect(repair).toHaveBeenCalledTimes(2)
  })

  it('repairs recreated toast proxies without touching unrelated hidden elements', async () => {
    const root = document.createElement('div')
    document.body.append(root)
    stops.push(observeUiAccessibility(root, exposeToastFocusGuards))
    for (let i = 0; i < 2; i++) {
      root.innerHTML =
        '<div><span aria-hidden="true" tabindex="0"></span><ol class="kchess-toasts"></ol><span aria-hidden="true" tabindex="0"></span></div><span aria-hidden="true" tabindex="0"></span>'
      await flush()
      expect(root.querySelectorAll(':scope > div > span[aria-hidden]')).toHaveLength(0)
      expect(root.querySelectorAll(':scope > span[aria-hidden]')).toHaveLength(1)
    }
  })

  it('does not rewrite an existing label or observe its own attribute repairs', async () => {
    const root = document.createElement('div')
    root.innerHTML =
      '<div class="kchess-search-palette"><div role="listbox" aria-label="Search results"></div></div>'
    const set = vi.spyOn(root.querySelector('[role="listbox"]')!, 'setAttribute')
    const repair = vi.fn(labelSearchResults)
    const stop = observeUiAccessibility(root, repair)
    stops.push(stop)
    await flush()
    expect(set).not.toHaveBeenCalled()
    expect(repair).toHaveBeenCalledTimes(1)
    stop()
    root.replaceChildren()
    await flush()
    expect(repair).toHaveBeenCalledTimes(1)
  })

  it('expands the top-bar search in place and closes it from outside or Escape', async () => {
    const onOpen = vi.fn()
    const palette = mount(AppCommandPalette, {
      attachTo: document.body,
      props: { open: false, 'onUpdate:open': onOpen },
      global: {
        stubs: {
          LazyUCommandPalette: {
            props: ['input'],
            template:
              '<div><input :aria-label="input[\'aria-label\']"><div role="listbox"></div></div>',
          },
          UKbd: true,
        },
      },
    })
    expect(palette.get('[role="listbox"]').attributes('aria-label')).toBe('Search results')
    expect(palette.get('input').attributes('aria-label')).toBe('Search pages and commands')
    // Focusing the box in the top bar is what opens it; there is no second search.
    await palette.get('input').trigger('focusin')
    expect(onOpen).toHaveBeenLastCalledWith(true)
    await palette.setProps({ open: true })
    await palette.get('input').trigger('keydown', { key: 'Escape' })
    expect(onOpen).toHaveBeenLastCalledWith(false)
    await palette.setProps({ open: true })
    onOpen.mockClear()
    palette.element.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(onOpen).not.toHaveBeenCalled()
    document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }))
    expect(onOpen).toHaveBeenLastCalledWith(false)
    const disconnect = vi.spyOn(MutationObserver.prototype, 'disconnect')
    palette.unmount()
    expect(disconnect).toHaveBeenCalledTimes(1)

    const toaster = mount(AppToaster, {
      global: {
        stubs: {
          UToaster: {
            name: 'UToaster',
            props: ['portal'],
            template:
              '<div><span aria-hidden="true" tabindex="0"></span><ol class="kchess-toasts"></ol></div>',
          },
        },
      },
    })
    expect(toaster.get('span').attributes('aria-hidden')).toBeUndefined()
    expect(toaster.findComponent({ name: 'UToaster' }).props('portal')).toBe(false)
    toaster.unmount()
    expect(disconnect).toHaveBeenCalledTimes(2)
  })
})
