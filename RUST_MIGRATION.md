# Migrating KChess to Rust — complete

**Complete (2026-10-29).** KChess is Rust only: the GPUI app (`apps/gpui`) replaced the Electron
app on every platform, the CLI (`apps/cli`) and the tooling (`tooling/xtask`) are Rust, and no
JavaScript, TypeScript or Node remains in the repository. This file keeps the end state and the
rules that came out of the migration and still hold. The phase procedures, the N-API and
WebAssembly bindings, the generated TypeScript contracts and the golden suites recorded from the
TypeScript implementation are in Git history (`git log -- RUST_MIGRATION.md`).

## End state

```text
crates/
  kchess-contracts/        The CoreApi method names, argument and event types (serde)
  kchess-domain/           Pure rules: chess, PGN, documents, sessions, validation (sync, no I/O)
  kchess-games/            Game sessions as frontends drive them (computer, local, archive)
  kchess-core/             Services: storage, SQLite, engines, Lichess, OAuth, logging (tokio)
  kchess-service/          The profile's shared core service (docs/shared-core.md)
  kchess-voice/            The speech helper (Vosk in its own process)
apps/
  gpui/                    The desktop app (GPUI Kit)
  cli/                     The command line
```

Frontends call the core by `CoreApi` method name (`kchess-contracts/src/core_api.rs`) and
subscribe to its events, usually through the shared core service; host capabilities they answer
(`openExternal`, `focus`, `onBattery`) arrive as `host:request` events. See `ARCHITECTURE.md` and
`CORE_FRONTENDS.md`.

## The bridge stays generic

The core is called as `call(method, args) → json`, by name, by design: one dispatch serves the
app, the CLI and the shared core service alike. Typing lands elsewhere: the method list in
`CoreApi`, the argument tables in `kchess-domain` `misc/contracts.rs` (a validator for every
argument; minimum arity, no excess arguments), and typed `arg<T>` / `json<T>` at each call site in
`kchess-core/src/core.rs`. Two tests pin the names a string can get wrong:
`kchess-core/tests/core/facade_dispatch.rs` (the facade answers exactly `CoreApi`) and
`kchess-core/tests/core/domain_dispatch.rs` (every `domain("…")` literal names a rule
`kchess_domain::api::call` answers).

## Stateful modules

Game and training sessions (computer, local, archive, online and puzzle games) keep every
decision in Rust transitions in `kchess-domain`; their drivers (`kchess-games`, the core's online
session) make none.

- **State** is a plain serializable object, owned by the driver.
- **Transitions** are pure functions `method(state, input, ctx) → { state, …, effects }`. `ctx`
  carries what the driver read from its host at that moment (`now`, `allowed`, `ready`,
  `activeAccount`, …), read once before the call. Getters (`over`, `winner`, `result`,
  `remaining`, …) are a `view(state, ctx)`.
- **Effects** are data the driver executes in order: host callbacks (`moved`, `flagged`,
  `notify`, `failed`), engine or network requests (with the epoch and count their reply must
  match) and timers. Async results come back through another transition
  (`searchFinished(state, {epoch, count, move | error}, ctx)`), so cancellation and stale-reply
  rules stay in the rules.
- The driver has no conditionals about chess or game rules: it reads ctx, calls the transition,
  keeps the returned state and runs the effects.

## Pinned behaviour

`kchess-domain/tests/rules_corpus.rs` pins every rules method to answers first recorded from the
TypeScript suites. Rewrite it (`KCHESS_WRITE_CORPUS=1 cargo test -p kchess-domain --test
rules_corpus`) only for an intended behaviour change. Behavioural tests of the public `CoreApi`
live in `crates/kchess-core/tests/`.

## Rules the core keeps

- Same observable behaviour as the core it replaced: results, error messages, event payloads,
  logging scope and level, cancellation, timeouts, bounds, file permissions (0600 data files), and
  AGENTS.md's architecture rules (Lichess stream authority, no retry of ambiguous move mutations,
  engine scheduler leases, `puzzles.db` single owner, append-only migrations, no secrets in logs).
- Expected cancellations stay distinguishable from failures (`SearchCancelled`, `AbortError`).
- No `unwrap()`/`expect()` on input or I/O paths (clippy denies them in libraries and shipped
  binaries); a panic in a task is caught and logged (`kchess-core/src/panics.rs`, `tasks.rs`).
- Logging goes through the host's log sink, with scopes and redaction (GUARDRAILS.md).
- Add a crate only with a reason in the commit message; licenses must be GPL-compatible
  (`deny.toml`).

## The facade

`crates/kchess-core/src/facade.rs` (and `facade/`) answers every `CoreApi` method: argument
contracts (`kchess_domain::misc::validate_arguments`), assistance gating during live games, login
and account epochs, settings that stop the engines, challenge and tournament bookkeeping, the
settings and data projections, the profile claim (one live core per
profile, released on close), the closed state and the close sequence.

- Close aborts in-flight work (an engine search, a Lichess request, a puzzle download) and waits
  for the facade calls it started; later calls fail with `The KChess core is closed.`.
- Facade calls run boxed futures: the method table is one large future, which overflowed a worker
  stack when inlined.
- Integral JSON numbers from the validators go out as integers.
