# Migrating the core to Rust

Goal: the headless core (`core/`) becomes Rust (`crates/`). When done, `core/` is deleted; no
hand-written TypeScript implements core behaviour. Frontends (Electron main and renderer, the
Node host, the CLI) stay TypeScript and reach the core only through its two bindings.

## End state

```text
crates/
  kchess-domain/   Pure rules: chess, PGN, documents, sessions, validation (sync, no I/O)
  kchess-core/     Services: storage, SQLite, engines, Lichess, OAuth, logging (tokio)
  kchess-node/     @kchess/native: N-API module; NativeCore (async calls + events) + rules
  kchess-wasm/     The renderer's sync rules (kchess-domain only)
```

- **Types** come from Rust: contract types derive `serde` and `ts_rs::TS`; their `.d.ts` files
  are generated (`pnpm run build:native`) and checked in under `crates/kchess-node/types/`
  as generated output. `CoreApi`, `CoreEvents`, `CORE_METHODS` and settings defaults are
  generated the same way.
- **Bindings** are thin and generic: `NativeCore.call(method, argsJson) → Promise<json>`,
  events through one JS callback `(event, json)`, host capabilities (`SecretStore`,
  `openExternal`, `focus`, `onBattery`, log sink) as JS callbacks the core awaits. The
  renderer keeps calling `invoke(method, argsJson)` on the WebAssembly module. The small
  typed wrappers (`Position`, `rules()`) move next to their binding as generated or binding
  code, not core code.

## How each piece moves (the strangler pattern)

The app must work and `pnpm run check` must pass after every step. A TypeScript service is
replaced in one commit:

1. Implement it in Rust behind the bridge (`kchess-core`), owning the same files, database
   tables, events and error messages. On-disk formats never change during the migration.
2. Port its tests. Behavioural tests of the public `CoreApi` keep running unchanged against the
   native core (they move to `tests/core/` once `core/` is gone). Tests of internals become Rust
   `#[test]`s with the same cases and assertions. Pure functions are first pinned to
   `core/tests/unit/native-golden.json` from the TypeScript implementation
   (`KCHESS_WRITE_GOLDEN=1`), then deleted from TypeScript.
3. Switch the TypeScript caller to the bridge and delete the TypeScript implementation.
4. `pnpm run check`, `pnpm run check:rust`; for anything the app exercises,
   `pnpm run build && pnpm run test:e2e`. Commit.

While services are split between the languages, the Rust core reaches remaining TypeScript
services only through the bridge's host callbacks; TypeScript reaches Rust only through
`core/src/services/rules.ts` (rules) and `core/src/services/nativeCore.ts` (services).

## Rules every port keeps

- Same observable behaviour: results, error messages, event payloads, logging scope/level,
  cancellation, timeouts, bounds, file permissions (0600 data files), and AGENTS.md's
  architecture rules (Lichess stream authority, no retry of ambiguous move mutations, engine
  scheduler leases, `puzzles.db` single owner, append-only migrations, no secrets in logs).
- Errors cross the bridge as `Error(message)`; expected cancellations keep `name = 'AbortError'`.
- No `unwrap()`/`expect()` on input or I/O paths; panics are caught at the bridge.
- Logging goes through the host log sink with the TypeScript scopes and redaction.
- Crates: `tokio`, `rusqlite` (bundled), `reqwest` (rustls, stream), `zstd`, `ts-rs`, `serde`.
  Add others only with a reason in the commit message; licenses must be GPL-compatible.

## Phases

| #   | Scope                                                                                                                                                                                                                                                                        | Status |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| 0   | Rules, documents, positions, PGN (`kchess-domain`)                                                                                                                                                                                                                           | done   |
| 1   | Bridge (`kchess-core`, `NativeCore`), puzzle database and queries                                                                                                                                                                                                            | done   |
| 2a  | Pure domain: voice, board editor, coordinates, training (rush, puzzles, endgames, openings, clock, engine levels, UCI info, coach), records (reviews, ratings, studies, results, time controls, online events, Lichess errors, OAuth look), validation and library documents | done   |
| 2b  | Stateful domain: game sessions and archive, online game, puzzle sessions, analysis-tree edits; remaining TS constants (`patterns.ts`, `tvChannels.ts`, grammar and data constants pinned by golden suites)                                                                   | done   |
| 3   | Storage: settings, accounts, game store, library, review store, migrations                                                                                                                                                                                                   | done   |
| 4   | Engines: UCI controller, scheduler, managed Stockfish, analysis, reviews. The core owns one engine context and the services; TypeScript wraps them (Lichess analysis for reviews is a transitional host request, `lichess.reviews`)                                          | done   |
| 5   | Lichess: client, OAuth, usage/request policy, online games, challenges, tournaments, spectate, studies, cloud eval, explorer                                                                                                                                                 | done   |
| 6   | Facade (`service.ts`), contracts and logging to Rust; generated types; delete `core/`. Replace the TypeScript coverage floor (`vitest.config.ts`) with a Rust coverage gate (cargo-llvm-cov) once the core's TypeScript is gone                                              |        |

## Porting a domain module (phase 2 procedure)

For each TypeScript module in `core/src/domain` assigned to you:

1. **Rust.** Implement every exported function and constant in the Rust file of your group
   (`crates/kchess-domain/src/{voice,training,records,misc}.rs`, or a submodule declared from
   it). Data tables stay data (a `const`/`static` or `include_str!` of the existing JSON).
   Expose each function through your group's `call(method, args)` with the TypeScript name as
   the method name (`"spokenMove" => Some(…)`), JSON arguments and result. Serde structs mirror
   the TypeScript shapes exactly (`rename_all = "camelCase"`, absent optionals skipped,
   `undefined` results as `null`). Mirror JavaScript semantics precisely: `Math.round`,
   `toFixed`, string `split`/`trim`/`toLowerCase` on Unicode, regexes (no regex crate in
   `kchess-domain`; hand-write matchers), integer vs. float formatting (`crate::js` has helpers).
   Randomness comes in as an argument (a number in [0, 1) or a list of them) — the TypeScript
   wrapper passes `Math.random()` values — so the Rust stays deterministic.
2. **Golden.** Write `core/tests/unit/native-<group>.test.ts` using `goldenFile('<group>')` from
   `core/tests/unit/golden.ts`. For each function, generate many cases (seeded `rng` like
   `native-rules.test.ts`, plus hand-picked edge cases, plus every input the existing tests use)
   and call `golden.check(suite, i, rust, typescript)` where `rust = rules('<name>', ...)` and
   `typescript` is the current TS function's result, and also `expect(rust).toEqual(plain(ts))`.
   Run with `KCHESS_WRITE_GOLDEN=1` once to record, then without it to confirm.
3. **Switch.** Replace each TypeScript function body with a thin wrapper over
   `rules('<name>', ...)` (import `rules` from `./engine.ts`); keep the exported names, types
   and signatures so no caller changes. Stateful classes keep their public API but hold plain
   state and call pure Rust transitions (`state in, state out`). Then remove the TypeScript
   reference from your golden test (keep the golden checks only) and confirm it still passes.
4. **Verify** in your worktree: `pnpm install`, `pnpm run build:native`, `cargo test -p
kchess-domain`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all`,
   `npx vitest run core/tests/unit apps/desktop/tests/unit`, `pnpm run typecheck`,
   `pnpm run lint`, `pnpm run format:check`. Commit on your branch with a descriptive message.

## Porting a stateful module (phase 2b pattern)

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
- **Golden traces:** before switching, record sequences of method calls with scripted host
  responses (seeded, including races: stale epochs, disposal mid-search, availability changes,
  clock expiry) and their observable outputs (state snapshots, return values, host calls in
  order). Record them from the TypeScript class, then check the Rust-backed class against them.
  Every existing test of the class must keep passing unchanged.

## Phase 5 switch checklist

Done. Each item is fixed in Rust and pinned by the named test.

- Reviews found during a game sync are saved with their page of games in one transaction
  (`saveGamesPage`): `lichess_accounts::a_synced_page_saves_its_reviews_with_its_games`.
  `LichessStore::save_reviews` serves the review queue's own fetch.
- Broadcast round PGN is split at the two blank lines that end each game, as `readPgnStream`
  split it, and one `watch:broadcast` is sent per chunk:
  `lichess_watch::a_round_feed_is_sent_per_chunk_with_the_games_that_have_arrived`.
- NDJSON parse errors surface when the line arrives (the line callback is fallible, so a failed
  line ends the read): `stream::a_failed_line_ends_the_read_before_the_stream_does`,
  `lichess_online::a_malformed_line_on_the_event_stream_is_reported_as_it_arrives`,
  `lichess_studies::a_malformed_study_record_fails_the_listing_as_it_arrives`. TV feed records
  are read the same way.
- TypeScript messages the UI shows are kept (`Lichess sent a puzzle KChess could not read.`,
  `Position lookup failed. Check your connection and retry.`, `Could not refresh. Showing the saved
lookup.`, `Lichess did not create the study.`). Unreadable JSON records keep the Rust parser's
  message, not V8's `JSON.parse` text.
- Endpoint tests: `lichess_endpoints` covers `tv_channels`, `broadcast_tour`, `puzzle_daily`,
  `puzzle_dashboard`, `puzzle_activity` and `storm_dashboard`.
- **Must fix:** a challenge cancelled while its creation is in flight waits for Lichess's answer
  and cancels the late challenge id: `lichess_online::aborts_an_outstanding_challenge_and_sends_no_more_once_cancelled`.
- The challenge event reader is in `kchess-domain` (`records/challenge.rs`, rule
  `readChallengeEvent`), beside `validateOnlineEvent`; `lichess::challenges` tests cover it.
- Usage: the core batches `host:usage` counters in `usage.rs` (flushed every five seconds, or
  before a report) and writes the same rows through `store.usage.flushUsage`; the TypeScript
  batching is gone: `usage::tests`.
