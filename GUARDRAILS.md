# Guardrails and verification

Run `cargo xtask check-fast` while editing. It checks format and lints for the workspace and runs
the tests of every package with changes since `main` (committed, staged, unstaged or new); a
change to `Cargo.toml` or `Cargo.lock` runs every test. This is an editing aid; related tests do
not establish complete regression coverage.

The app's end-to-end tests (`apps/gpui/src/e2e`) run the whole app headless over a fresh profile
with the scripted engine: `cargo test -p kchess-gpui e2e::`, or one area with `e2e::computer`.
They run with the app's unit tests in `cargo xtask check`, which CI runs on Linux, and in CI's
`gpui` job on macOS and Windows.

`e2e::audit` opens every page at the minimum, a mid and the default window size and fails when a
control has no accessible name, runs past the window's side or overlaps another control, or when
more controls fall under 24 px than its ratchet allows (lower it as they grow). For UI changes,
compare pictures of every page before and after: on macOS,
`KCHESS_SCREENSHOTS=<dir> cargo test -p kchess-gpui e2e::screens -- --ignored` renders them
offscreen into `<dir>/<width>x<height>/<page>.png`.

`cargo xtask install-hooks` points Git at `.githooks`. Its pre-push hook runs the full
`cargo xtask check` when a push updates `main`, and refuses if the pushed commit is not HEAD or
the working tree has uncommitted or untracked files, since `check` would test those instead. Other
branches push without it. Skip it only on purpose: `KCHESS_SKIP_PRE_PUSH=1 git push …`.

Before merging, run `cargo xtask check`: `cargo fmt --check`, clippy over every target with
warnings denied (and `unwrap`/`expect` denied in libraries and shipped binaries), the notices
check, the release metadata, and every test, under the coverage gate when `cargo-llvm-cov` is
installed. Packaging changes also need `cargo xtask package` on the platform they touch. CI runs
`check`, `cargo deny`, and the app's tests on macOS, Windows and Linux; when packaging may have
changed since `main` last packaged successfully (a version bump always has), it also packages for
macOS and Linux, builds the Snap and runs the Windows upgrade workflow, which packages for Windows.
Only a release commit's packages use the release profile's fat LTO; the others link without it.
A change to documents alone (`docs/` and the top-level guides) runs none of it. `cargo xtask release-verify` refuses a commit
whose CI did not run and pass every job, and every push to `main` runs CI to the end.

## Ownership

- The core (`crates/kchess-core`) never depends on a frontend. Frontends reach it by `CoreApi`
  method name, through the profile's core service or an embedded core; host capabilities they
  answer (`openExternal`, `focus`, `onBattery`) arrive as `host:request` events.
- Every core method has a row in the argument table (`crates/kchess-domain/src/misc/contracts.rs`)
  naming a validator for every argument, including optional ones; it declares the minimum arity
  and rejects excess arguments before any check runs. Domain and service validation still runs
  after. `tests/facade_dispatch.rs` checks the facade answers exactly the methods of `CoreApi`.
- The Rust puzzle service is the only owner of `puzzles.db`; the core's store
  (`crates/kchess-core/src/store`) is the only owner of `kchess.db`.

## Logging: never swallow errors

- Every error is either returned or logged with context through the core's host
  (`Host::log`, `Level::Debug/Info/Warn/Error`, with a scope such as `engine` or `lichess`):
  debug for expected or benign outcomes (cancel teardown, cache miss), warn for recoverable ones
  (cache write, reconnect, throttle), error for failures needing attention.
- Never log tokens, passwords, request bodies or PGNs; the diagnostics export redacts Bearer
  tokens, `lip_`/`lio_` keys, OAuth codes and emails. Cancellations log at debug.

## Lifecycle ownership

Replaceable work carries an epoch or generation, checked after every await before its answer is
adopted (the app's pages do this for every lookup, read and search). Use `EngineContext::lease`
(`crates/kchess-core/src/engine/mod.rs`) for engine configuration and search; it releases the
scheduler on success, failure and cancellation. `UciController` (`engine/uci.rs`) owns process
deadlines and UCI pipes.

For each escaped regression, preserve the reproducer and test its sequence: late replies,
overlapping requests, cancellation, lost credentials, interrupted writes and recovery. When a
generated or fuzzed case finds a defect, keep its seed and a focused reproducer.

Timing races in the app's tests show up under load: `cargo test -p kchess-gpui -- --test-threads=40`
runs the suite oversubscribed and should pass as reliably as a plain run. The harness runs with
reduced motion, so dialogs and popovers are at rest when a test clicks them.

## Coverage and corpora

- `cargo xtask check` runs `cargo llvm-cov` over the workspace (without the app's views and the
  tooling, whose own tests still run) and fails below the floors in `tooling/xtask/src/check.rs`,
  which are rounded down from the measured totals and dated there. Its summary is kept in
  `test-results/rust-coverage/summary.json` and uploaded with CI's measurements. Ratchet the
  floors up when the totals rise; lower them only with a measurement that explains the drop.
- `crates/kchess-domain/tests/rules_corpus.rs` replays 32k rules calls recorded from the former
  TypeScript suites; regenerate it only for an intended behaviour change.
- `fuzz/` holds cargo-fuzz targets for PGN, FEN, UCI output, online events and the rules API
  (`.github/workflows/fuzz.yml`).
