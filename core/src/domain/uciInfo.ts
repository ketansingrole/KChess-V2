import type { EngineLine } from '../contracts/types.ts'
import { UCI_MOVE } from './patterns.ts'

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
  // Hot path: hundreds of `info` lines per second during a search, plus
  // `bestmove`/progress lines that carry no score. The exact `startsWith`
  // check avoids a trim+split for every non-`info` line; indented input falls
  // through to the trimmed path below so semantics never change.
  if (!text.startsWith('info')) {
    const first = text[0]
    if (first !== ' ' && first !== '\t' && first !== '\n' && first !== '\r') return undefined
  }
  const tokens = text.trim().split(/\s+/)
  if (tokens[0] !== 'info') return undefined
  let depth: number | undefined
  let rank = 1
  let cp: number | undefined
  let mate: number | undefined
  let nps: number | undefined
  let pv: string[] | undefined
  for (let i = 1; i < tokens.length; i++) {
    const token = tokens[i]
    const value = tokens[i + 1]
    if (token === 'depth') depth = Number(value)
    else if (token === 'multipv') rank = Number(value)
    else if (token === 'nps') nps = Number(value)
    else if (token === 'score') {
      const kind = value
      const amount = Number(tokens[i + 2])
      if (kind === 'cp') cp = amount
      else if (kind === 'mate') mate = amount
      i += 2
      // A lower/upper bound is only a provisional score from a fail-high/low: skip the line.
      if (tokens[i + 1] === 'lowerbound' || tokens[i + 1] === 'upperbound') return undefined
      continue
    } else if (token === 'pv') {
      pv = []
      for (let j = i + 1; j < tokens.length; j++) {
        const move = tokens[j]!
        if (UCI_MOVE.test(move)) pv.push(move)
      }
      break
    }
    // `string` is free text with no pv; `wdl` takes three values; every other key takes one.
    if (token === 'string') return undefined
    i += token === 'wdl' ? 3 : 1
  }
  if (depth === undefined || !Number.isFinite(depth) || !pv?.length) return undefined
  if (cp === undefined && mate === undefined) return undefined
  const sign = whiteToMove ? 1 : -1
  const line: EngineLine = { rank, depth, pv }
  if (mate !== undefined && Number.isFinite(mate)) line.mate = mate * sign
  else if (cp !== undefined && Number.isFinite(cp)) line.cp = cp * sign
  else return undefined
  return { line, nps: nps !== undefined && Number.isFinite(nps) ? nps : undefined }
}
