# Migrating KChess to Rust — complete

**Complete (2026-10-29).** KChess is Rust only: the GPUI app (`apps/gpui`) replaced the Electron
app on every platform, the CLI (`apps/cli`) and the tooling (`tooling/xtask`) are Rust, and no
JavaScript, TypeScript or Node remains in the repository. This file keeps the rules that came out
of the migration and still hold. The phase procedures, the N-API and WebAssembly bindings, the
generated TypeScript contracts and the golden suites recorded from the TypeScript implementation
are in Git history (`git log -- RUST_MIGRATION.md`). The workspace is mapped in `ARCHITECTURE.md`,
and how frontends reach the core is in `CORE_FRONTENDS.md`.

## One dispatch, typed at both ends

On the wire a call is still `call(method, args) → json`: one dispatch serves the app, the CLI and
the shared core service alike. Each method is one `core_api!` declaration in `kchess-contracts`
(`core_api.rs`), with a check for every argument (minimum arity, no excess arguments); it makes
the `CoreApi` traits the core implements, the dispatch that runs a call by name on them, and the
typed call frontends make (`kchess_contracts::calls`). `kchess-core/tests/core/facade_dispatch.rs`
pins the names a string can get wrong: the core answers exactly `METHODS`.

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

`crates/kchess-core/src/facade.rs` (and `facade/`) answers every `CoreApi` method once the
generated dispatch has checked its arguments (each argument's type, `kchess_contracts::check`) and, for a method
declared with `#[assist(..)]`, refused engine help during a live game: login and account epochs,
settings that stop the engines, challenge and tournament bookkeeping, the settings and data
projections, the closed state and the close sequence. The profile claim (one live core per
profile, released on close) is `profile_lock.rs`.

- Close aborts in-flight work (an engine search, a Lichess request, a puzzle download) and waits
  for the facade calls it started; later calls fail with `The KChess core is closed.`.
- The dispatch runs each method as a boxed future (`kchess-contracts/src/api.rs`): the method
  table inlined as one future overflowed a worker stack.
