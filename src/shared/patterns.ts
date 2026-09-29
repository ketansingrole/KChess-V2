/** Formats shared by IPC validation (main) and game handling (renderer). */

export const USERNAME = /^[a-zA-Z0-9_-]{2,30}$/
export const UCI_MOVE = /^[a-h][1-8][a-h][1-8][qrbn]?$/
export const GAME_ID = /^[a-zA-Z0-9]{8,12}$/
