# Migrating the core to Rust — complete

**Complete (2026-10-29).** KChess is Rust only: the GPUI app replaced the Electron app on every
platform, the CLI and tooling are Rust, and no JavaScript, TypeScript or Node remains. What follows
is the record of how the core got there.

> Status (2026-10-13): complete. Phases 0–6 are done and `core/` is deleted. This file keeps the
> end state and the rules that still hold; the phase procedures and checklists are in Git history
> (`git log -- RUST_MIGRATION.md`).

The headless core is Rust (`crates/`); no hand-written TypeScript implements core behaviour.
Frontends (Electron main and renderer, the Node host, the CLI) stay TypeScript and reach the core
only through its two bindings.

## End state

```text
crates/
  kchess-domain/   Pure rules: chess, PGN, documents, sessions, validation (sync, no I/O)
  kchess-core/     Services: storage, SQLite, engines, Lichess, OAuth, logging (tokio)
  kchess-node/     @kchess/native: N-API module; NativeCore (async calls + events) + rules
  kchess-wasm/     The renderer's sync rules (kchess-domain only)
```

- **Types** come from Rust: contract types derive `serde` and `ts_rs::TS`; their `.d.ts` files
  are generated (`pnpm run build:native`) and checked in under `crates/kchess-contracts/ts/generated/`
  as generated output. `CoreApi`, `CoreEvents`, `CORE_METHODS` and settings defaults are
  generated the same way.
- **Bindings** are thin and generic: `NativeCore.call(method, argsJson) → Promise<json>`,
  events through one JS callback `(event, json)`, host capabilities (`SecretStore`,
  `openExternal`, `focus`, `onBattery`, log sink) as JS callbacks the core awaits. The
  renderer keeps calling `invoke(method, argsJson)` on the WebAssembly module. The small
  typed wrappers (`Position`, `rules()`) live next to their binding as generated or binding
  code, not core code.

## End-state decision (2026-10-10)

`core/` is deleted. Its TypeScript is three packages: `@kchess/contracts` (`crates/kchess-contracts/ts`,
the generated types and IPC validators), `@kchess/rules` (`crates/kchess-wasm/js`, the rules wrappers)
and `@kchess/native` (`crates/kchess-node/js`, the binding glue). Contract types are generated from Rust.
The only hand-written TypeScript left for the core is binding glue inside the binding packages (`crates/kchess-node` for Node/Electron
main, `crates/kchess-wasm` for the renderer): forwarding calls, answering host capability
requests, running effects returned by Rust transitions, and keeping state reactive for Vue. Glue
makes no decisions; any conditional about chess, games, storage, engines or Lichess belongs in
Rust. Rust frontends (GPUI, CLI, TUI, MCP) use the crates directly and need no glue.

## Golden tests

Rules ported from TypeScript were pinned first to `tests/core/native-golden.json` (and the
per-group suites read through `tests/core/golden.ts`) from the TypeScript implementation, then the
TypeScript was deleted. Those files now pin the Rust behaviour; rewrite them
(`KCHESS_WRITE_GOLDEN=1`) only for an intended change. Behavioural tests of the public `CoreApi`
live in `tests/core/`.

## Rules the Rust core keeps

- Same observable behaviour: results, error messages, event payloads, logging scope/level,
  cancellation, timeouts, bounds, file permissions (0600 data files), and AGENTS.md's
  architecture rules (Lichess stream authority, no retry of ambiguous move mutations, engine
  scheduler leases, `puzzles.db` single owner, append-only migrations, no secrets in logs).
- Errors cross the bridge as `Error(message)`; expected cancellations keep `name = 'AbortError'`.
- No `unwrap()`/`expect()` on input or I/O paths; panics are caught at the bridge.
- Logging goes through the host log sink with the TypeScript scopes and redaction.
- Crates: `tokio`, `rusqlite` (bundled), `reqwest` (rustls, stream), `ruzstd`, `ts-rs`, `serde`.
  Add others only with a reason in the commit message; licenses must be GPL-compatible.

## History

0 rules and PGN (`kchess-domain`); 1 the bridge and puzzle database; 2a pure domain modules;
2b stateful sessions; 3 storage; 4 engines; 5 Lichess; 6a the facade in Rust; 6b `core/`
deleted, generated contracts, and the Rust coverage gate (`pnpm run check:rust-coverage`). The bridge
`NativeCore.call(method, argsJson)` stays generic by design: typing lands in the generated
`CoreApi`/`CoreEvents`/`CORE_METHODS`, the argument tables in `kchess-domain` `misc/contracts.rs`,
and typed `arg<T>` / `json<T>` at each call site in `kchess-core/src/core.rs`.

## Stateful modules (the phase 2b pattern)

Session classes (`ComputerGame`, `LocalGame`, `GameArchive`, `OnlineGame`, `PuzzleSession`) keep
their exported TypeScript API, but every decision moves to Rust:

- **State** is a plain serializable object (the existing `*State` plus any private fields the
  class kept, such as `turnStarted`, `permitted`, `disposed`), owned by the TypeScript object so
  it stays reactive where hosts make it reactive.
- **Transitions** are pure Rust functions `method(state, input, ctx) → { state, …, effects }`.
  `ctx` carries what the class read from its host at that moment (`now`, `allowed`, `ready`,
  `activeAccount`, …) — read it once, in the same order the TypeScript did, before the call.
  Getters (`over`, `winner`, `result`, `remaining`, …) become a Rust `view(state, ctx)`.
- **Effects** are data the TypeScript driver executes in order: host callbacks (`moved`,
  `flagged`, `notify`, `failed`), engine/network requests (with the epoch/count they must be
  matched against), timers. Async results come back through another transition
  (`searchFinished(state, {epoch, count, move | error}, ctx)`), so cancellation and stale-reply
  rules stay in Rust.
- The driver has no conditionals about chess or game rules: it reads ctx, calls Rust, assigns
  the returned state back into the existing state object (keep object identity), and runs
  effects. Promise plumbing (`this.search` reuse, `void` fire-and-forget, `console.debug` of
  cancelled replies) stays exactly as before.
- **Golden traces:** when a class moved, we recorded sequences of method calls with scripted host
  responses (seeded, including races: stale epochs, disposal mid-search, availability changes,
  clock expiry) and their observable outputs (state snapshots, return values, host calls in
  order). They were recorded from the TypeScript class and now check the Rust-backed class.

## The facade

`crates/kchess-core/src/facade.rs` answers every `CORE_METHODS` entry and the `KChessCore`
extras (`settings`, `desktopSettings`, `saveDesktopSettings`, `trustEnginePath`,
`voiceHistoryDocument`, `suspend`, `close`) with the decisions `service.ts` made: argument contracts
(`kchess_domain::misc::validate_arguments`, the Rust form of `CORE_CONTRACTS`), assistance gating
during live games, login and account epochs, settings that stop the engines, challenge and
tournament bookkeeping, the settings and data projections, the one-time migration with its token
encryption, the profile claim (one live core per profile, released on close), the closed state and
the close sequence. `createKChessCore` builds the native core and forwards each method, generated
from `CORE_METHODS`; events forward through one sink, so the TypeScript keeps no event list.

- Close aborts in-flight work (an engine search, a Lichess request, a puzzle download) and waits for
  the facade calls it started; later calls fail with `The KChess core is closed.`: `core_close`.
- Facade calls run boxed futures: the method table is one large future, which overflowed a worker
  stack when inlined.
- Integral JSON numbers from the validators go out as integers (as `JSON.stringify` writes them).

Still TypeScript in `crates/kchess-node/js`, and why: `nativeCore.ts` (the binding, host capabilities
and the event sink), `service.ts` (the forwarder), `serviceCore.ts` (the client of the shared core service), `platform.ts` (the host scope), `logger.ts`,
`diagnosticLog.ts`, `performance.ts` (the diagnostics the desktop shell and the hosts use), `rules.ts`
and `native.ts` (the rules binding), `oauthPage.ts` and `stockfishAsset.ts` (pure helpers the desktop
tests use), `appIconSvg.ts`, `engine.ts` (the bundled engine's path and `SearchCancelled`), and the
voice model modules (not part of this phase). `crates/kchess-contracts/ts/apiContracts.ts` and
`apps/desktop/contracts/apiContracts.ts` only forward argument checks to Rust (`validateCoreArguments`,
`validateIpcArguments`). Phase 6b deleted `core/`; its TypeScript now lives in the three packages above.
