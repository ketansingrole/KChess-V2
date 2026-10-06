/** Quick-pick search: prefix modes and a VS Code-style fuzzy matcher with highlight ranges. */

export type QuickPickMode = 'all' | 'commands' | 'players' | 'settings' | 'help'

export const QUICK_PICK_PREFIXES: Record<string, QuickPickMode> = {
  '>': 'commands',
  '@': 'players',
  '#': 'settings',
  '?': 'help',
}

/** Split typed text into its mode (from a leading prefix character) and the text to match. */
export function parseQuickPick(raw: string): { mode: QuickPickMode; prefix: string; text: string } {
  const prefix = raw.charAt(0)
  const mode = QUICK_PICK_PREFIXES[prefix]
  return mode
    ? { mode, prefix, text: raw.slice(1).trim() }
    : { mode: 'all', prefix: '', text: raw.trim() }
}

export interface QuickPickMatch {
  score: number
  /** Inclusive [start, end] ranges of the label to highlight. */
  ranges: [number, number][]
}

const WORD_BREAK = /[\s·›/@:,()+-]/

function isWordStart(text: string, index: number): boolean {
  return index === 0 || WORD_BREAK.test(text.charAt(index - 1))
}

/** Match one token in `text`: whole substrings beat acronym-style subsequences, word starts beat the middle. */
function matchToken(token: string, text: string): QuickPickMatch | null {
  const lower = text.toLowerCase()
  let fallback = -1
  for (let at = lower.indexOf(token); at !== -1; at = lower.indexOf(token, at + 1)) {
    if (isWordStart(text, at)) {
      return {
        score: 100 + token.length * 4 + (at === 0 ? 40 : 0),
        ranges: [[at, at + token.length - 1]],
      }
    }
    if (fallback === -1) fallback = at
  }
  if (fallback !== -1 && token.length > 1) {
    return { score: 50 + token.length * 2, ranges: [[fallback, fallback + token.length - 1]] }
  }
  // Subsequence that starts at a word: "otb" → Over The Board, "sfb" → Stockfish … Black.
  const ranges: [number, number][] = []
  let score = 20
  let from = 0
  for (const char of token) {
    let at = -1
    // Prefer the next word start holding this character, else the next occurrence.
    for (let i = from; i < lower.length; i++) {
      if (lower[i] === char && isWordStart(text, i)) {
        at = i
        break
      }
    }
    const next = lower.indexOf(char, from)
    const contiguous = ranges.length > 0 && next === from
    if (contiguous || at === -1) at = next
    if (at === -1) return null
    if (!ranges.length && !isWordStart(text, at)) return null
    const last = ranges.at(-1)
    if (last && last[1] === at - 1) {
      last[1] = at
      score += 6
    } else {
      ranges.push([at, at])
      score += isWordStart(text, at) ? 8 : 0
    }
    from = at + 1
  }
  return { score, ranges }
}

/**
 * Every space-separated token must match the label or the extra search text (keywords,
 * details). Only label matches are highlighted and they score higher.
 */
export function matchQuickPick(query: string, label: string, extra = ''): QuickPickMatch | null {
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (!tokens.length) return { score: 0, ranges: [] }
  let score = 0
  const ranges: [number, number][] = []
  for (const token of tokens) {
    const inLabel = matchToken(token, label)
    if (inLabel) {
      score += inLabel.score
      ranges.push(...inLabel.ranges)
      continue
    }
    const inExtra = extra ? matchToken(token, extra) : null
    if (!inExtra) return null
    score += inExtra.score / 2
  }
  return { score, ranges: mergeRanges(ranges) }
}

function mergeRanges(ranges: [number, number][]): [number, number][] {
  const sorted = [...ranges].sort((a, b) => a[0] - b[0])
  const merged: [number, number][] = []
  for (const range of sorted) {
    const last = merged.at(-1)
    if (last && range[0] <= last[1] + 1) last[1] = Math.max(last[1], range[1])
    else merged.push([...range])
  }
  return merged
}

const ESCAPES: Record<string, string> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
}
export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (char) => ESCAPES[char]!)
}

/** Escaped label HTML with matched ranges wrapped in `<mark>`. */
export function highlightRanges(label: string, ranges: [number, number][]): string {
  let html = ''
  let from = 0
  for (const [start, end] of ranges) {
    html += escapeHtml(label.slice(from, start))
    html += `<mark>${escapeHtml(label.slice(start, end + 1))}</mark>`
    from = end + 1
  }
  return html + escapeHtml(label.slice(from))
}

/** Rank items by match score, keeping catalog order for ties. */
export function rankQuickPick<T extends { label: string; keywords?: string; detail?: string }>(
  query: string,
  items: readonly T[],
): { item: T; match: QuickPickMatch }[] {
  return items
    .map((item, index) => ({
      item,
      index,
      match: matchQuickPick(query, item.label, `${item.detail ?? ''} ${item.keywords ?? ''}`),
    }))
    .filter((entry): entry is { item: T; index: number; match: QuickPickMatch } =>
      Boolean(entry.match),
    )
    .sort((a, b) => b.match.score - a.match.score || a.index - b.index)
    .map(({ item, match }) => ({ item, match }))
}

/** Most recent first, without duplicates, capped. */
export function pushRecent(recent: readonly string[], id: string, limit = 8): string[] {
  return [id, ...recent.filter((entry) => entry !== id)].slice(0, limit)
}
