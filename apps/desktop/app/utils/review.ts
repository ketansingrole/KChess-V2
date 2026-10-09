import type { Judgment, ReviewSide } from '@kchess/core/contracts/types'

const WORDS: Record<Judgment, [string, string]> = {
  blunder: ['blunder', 'blunders'],
  mistake: ['mistake', 'mistakes'],
  inaccuracy: ['inaccuracy', 'inaccuracies'],
}

/** “2 blunders”, worst first, leaving out the kinds that did not happen. */
export function judgmentCounts(side: ReviewSide): { judgment: Judgment; text: string }[] {
  return (['blunder', 'mistake', 'inaccuracy'] as const)
    .filter((judgment) => side[judgment] > 0)
    .map((judgment) => ({
      judgment,
      text: `${side[judgment]} ${WORDS[judgment][side[judgment] === 1 ? 0 : 1]}`,
    }))
}
