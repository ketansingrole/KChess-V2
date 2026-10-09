import type { EngineLine } from '../contracts/types.ts'
import { rules } from './engine.ts'

export interface ParsedInfo {
  line: EngineLine
  nps?: number
}

/**
 * Read one UCI `info` line that carries a scored principal variation, turning the score to
 * White's point of view (UCI reports it for the side to move). Progress-only lines
 * (`currmove`, `hashfull` …) and bound scores from an unfinished iteration return undefined.
 */
export function parseInfo(text: string, whiteToMove: boolean): ParsedInfo | undefined {
  const parsed = rules<{ line: EngineLine & { rank: number | string }; nps?: number } | null>(
    'parseInfo',
    text,
    whiteToMove,
  )
  if (!parsed) return undefined
  // A rank that is not a finite number travels as its text.
  parsed.line.rank = Number(parsed.line.rank)
  return parsed as ParsedInfo
}
