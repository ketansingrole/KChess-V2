/** Nuxt UI does not currently forward a label to CommandPalette's inner listbox. */
export function labelSearchResults(root: ParentNode): void {
  for (const listbox of root.querySelectorAll('.kchess-search-palette [role="listbox"]')) {
    listbox.setAttribute('aria-label', 'Search results')
  }
}

/** Reka's toast focus proxies must stay keyboard reachable without being aria-hidden. */
export function exposeToastFocusGuards(root: ParentNode): void {
  for (const viewport of root.querySelectorAll('.kchess-toasts')) {
    const container = viewport.parentElement
    if (!container) continue
    for (const guard of container.querySelectorAll(
      ':scope > span[aria-hidden="true"][tabindex="0"]',
    )) {
      guard.removeAttribute('aria-hidden')
    }
  }
}
