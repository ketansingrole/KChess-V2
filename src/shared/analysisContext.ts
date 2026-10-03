export const analysisContext = (rootFen: string, moves: readonly string[] = []): string =>
  `${rootFen}|${moves.join(' ')}`
