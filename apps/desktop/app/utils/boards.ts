import { BOARD_LOOKS } from '@kchess/rules/looks'

/** Lichess 2D board themes, matching `ui/lib/css/theme/board/_boards.scss` in lila. */
export const boardThemes = BOARD_LOOKS.map(({ id, name }) => ({ id, name }))

export type BoardTheme = string
