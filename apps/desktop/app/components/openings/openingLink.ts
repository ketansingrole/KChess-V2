import type { RouteLocationRaw } from 'vue-router'

/** The openings page of a line: its UCI moves from the starting position. */
export function openingLink(moves: readonly string[]): RouteLocationRaw {
  return { path: '/openings', query: { line: moves.join(' ') } }
}

/** The line an openings page address names, or null for the tree of all openings. */
export function openingLine(query: unknown): string[] | null {
  return typeof query === 'string' ? query.split(/[\s+]+/).filter(Boolean) : null
}
