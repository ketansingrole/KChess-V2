/** Formats shared by IPC validation (main) and game handling (renderer). */

export const USERNAME = /^[a-zA-Z0-9_-]{2,30}$/
export const UCI_MOVE = /^[a-h][1-8][a-h][1-8][qrbn]?$/
export const GAME_ID = /^[a-zA-Z0-9]{8,12}$/
export const PUZZLE_ID = /^[a-zA-Z0-9]{3,12}$/
/** A puzzle theme key (`mateIn2`) or opening angle (`Sicilian_Defense`). */
export const PUZZLE_ANGLE = /^[a-zA-Z0-9_-]{1,60}$/
/** Loose shape of a FEN (piece placement, side, castling, en passant, optional counters). */
export const FEN =
  /^[1-8pnbrqkPNBRQK]+(?:\/[1-8pnbrqkPNBRQK]+){7} [wb] (?:[KQkqA-Ha-h]{1,4}|-) (?:[a-h][36]|-)(?: \d{1,3} \d{1,4})?$/
