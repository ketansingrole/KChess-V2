/** Shared restart warning for the update button and update settings. */
export const RESTART_UPDATE_DESCRIPTION =
  "KChess will close and reopen. Finish any game or timed puzzle run first; an unfinished local game or run will be lost, and an online game's clock will keep running."

/** This app's name in version labels, so a report says which KChess app it came from. */
export const APP_CLIENT = 'Electron'

/** `2026.10.3 · Electron`: the running version as the Updates heading and quick pick show it. */
export function clientVersion(version: string | undefined): string | undefined {
  return version ? `${version} · ${APP_CLIENT}` : undefined
}
