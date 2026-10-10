import { rulesLossless } from '@kchess/rules/engine'
import type { CoreMethod } from './core'
import { CORE_ARITY } from './generated/arity'

/** How many arguments a method takes: the fewest it accepts and the most (one check each). */
export interface ContractArity {
  readonly min: number
  readonly max: number
}

/** Argument counts from the generated arity table (`kchess_domain::misc::contracts`). */
export function arityContracts<K extends string>(
  table: Readonly<Record<K, readonly [number, number]>>,
): Readonly<Record<K, ContractArity>> {
  const entries = Object.entries(table) as [K, readonly [number, number]][]
  return Object.fromEntries(
    entries.map(([method, [min, max]]) => [method, { min, max }]),
  ) as Readonly<Record<K, ContractArity>>
}

/** Every headless invocation's argument counts. The checks run in Rust. */
export const CORE_CONTRACTS: Readonly<Record<CoreMethod, ContractArity>> =
  arityContracts(CORE_ARITY)

/**
 * Checks a core method's arguments in Rust (`validateCoreArguments`). Throws the first failing
 * check's message, as the TypeScript validators did; `undefined` in `args` is kept.
 */
export function validateCoreArguments(method: CoreMethod, args: unknown[]): void {
  rulesLossless('validateCoreArguments', method, Array.from(args))
}
