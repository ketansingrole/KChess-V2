/** Lichess's cloud evaluation of a position, for positions someone analysed deeply before. */
import type { CloudEval } from '../contracts/types'
import { nativeCall, nativeCallSync } from './nativeCore'

export function cloudEval(fen: string, lines: number): Promise<CloudEval | null> {
  return nativeCall('cloudEval.cloudEval', fen, lines)
}

export function clearCloudEval(): void {
  nativeCallSync('cloudEval.clear')
}
