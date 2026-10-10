/**
 * Lichess tournaments of the signed-in accounts: arenas and Swiss events, and joining, leaving and
 * creating them. The Rust core reads and writes them (`lichess/tournaments.rs`).
 */
import type {
  NeedsReconnect,
  NewArena,
  TournamentDetail,
  TournamentList,
  TournamentSummary,
  TournamentSystem,
} from '../contracts/types'
import { nativeCall } from './nativeCore'

/** The tournaments of an account's teams, with the problems that stopped part of them loading. */
export function tournaments(account: string): Promise<TournamentList> {
  return nativeCall('tournaments.list', account)
}

export function tournament(
  system: TournamentSystem,
  id: string,
  account: string,
  page = 1,
): Promise<TournamentDetail> {
  return nativeCall('tournaments.get', system, id, account, page)
}

export function joinTournament(
  system: TournamentSystem,
  id: string,
  account: string,
  password?: string,
): Promise<true | NeedsReconnect> {
  return nativeCall('tournaments.join', system, id, account, password ?? null)
}

export function leaveTournament(
  system: TournamentSystem,
  id: string,
  account: string,
): Promise<true | NeedsReconnect> {
  return nativeCall('tournaments.leave', system, id, account)
}

export function createTournament(
  account: string,
  arena: NewArena,
): Promise<TournamentSummary | NeedsReconnect> {
  return nativeCall('tournaments.create', account, arena)
}
