import { rules } from './engine.ts'

export const analysisContext = (rootFen: string, moves: readonly string[] = []): string =>
  rules<string>('analysisContext', rootFen, moves)
