import { rules } from './engine.ts'

/** A face of the chess clock: the player across the table, or the one in front of it. */
export type OtbSide = 'top' | 'bottom'

/** The times set before a session: the top player's minutes, the bottom's, and the increment. */
export interface OtbConfig {
  minutes: number
  bottomMinutes: number
  increment: number
}

/** The clock's state as the Rust rules keep it (`games/otb_clock.rs`); read it through them. */
export interface OtbClockState {
  config: OtbConfig
  running: OtbSide | null
  flagged: OtbSide | null
  moves: Record<OtbSide, number>
}

/** A transition's answer: the new state, and whether a flag fell instead. */
interface Moved {
  clock: OtbClockState
  flagFell: boolean
}

/** What Game history keeps of a session. */
export interface OtbArchive {
  hasPlay: boolean
  reason: string
  timeControl: string
  clockSummary: string
}

export const otbClockNew = (config: OtbConfig): OtbClockState =>
  rules<OtbClockState>('otbClockNew', config)

/** A face's time at `at`, in milliseconds. */
export const otbClockLeft = (clock: OtbClockState, side: OtbSide, at: number): number =>
  rules<number>('otbClockLeft', clock, side, at)

export const otbClockExpire = (clock: OtbClockState, at: number): Moved =>
  rules<Moved>('otbClockExpire', clock, at)

/** A player pressed their face: theirs stops (plus the increment), the other side's starts. */
export const otbClockPress = (clock: OtbClockState, side: OtbSide, at: number): Moved =>
  rules<Moved>('otbClockPress', clock, side, at)

export const otbClockPause = (clock: OtbClockState, at: number): Moved =>
  rules<Moved>('otbClockPause', clock, at)

/** Start (or resume) the clock; after a flag it begins a fresh session at the set times. */
export const otbClockStart = (clock: OtbClockState, at: number): Moved =>
  rules<Moved>('otbClockStart', clock, at)

export const otbClockReset = (clock: OtbClockState): OtbClockState =>
  rules<OtbClockState>('otbClockReset', clock)

/** New set times: a clock nobody has started shows them at once; a started one keeps its time. */
export const otbClockConfigure = (clock: OtbClockState, config: OtbConfig): OtbClockState =>
  rules<OtbClockState>('otbClockConfigure', clock, config)

export const otbClockArchive = (clock: OtbClockState): OtbArchive =>
  rules<OtbArchive>('otbClockArchive', clock)
