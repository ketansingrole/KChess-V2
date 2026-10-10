import { rulesLossless } from '@kchess/rules/engine'
import { arityContracts, type ContractArity } from '@kchess/contracts/apiContracts'
import { IPC_ARITY } from '@kchess/contracts/generated/arity'
import type { InvokeMethod } from './ipc'

/** Every IPC invocation's argument counts (the desktop's rows, then the core's). The checks run in Rust. */
export const IPC_CONTRACTS: Readonly<Record<InvokeMethod, ContractArity>> =
  arityContracts(IPC_ARITY)

/**
 * Checks an invocation's arguments in Rust (`validateIpcArguments`). Throws the first failing
 * check's message; `undefined` in `args` is kept, as the handler received it.
 */
export function validateIpcArguments(method: InvokeMethod, args: unknown[]): void {
  rulesLossless('validateIpcArguments', method, Array.from(args))
}
