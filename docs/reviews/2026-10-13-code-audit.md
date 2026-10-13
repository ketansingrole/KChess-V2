# Code and architecture audit — 2026-10-13

Scope: `main` at `65298e1` (five local commits ahead of `origin/main`, never run through CI).
Every stage of `pnpm run check` was run separately so one failure could not hide another.

| Stage                                                                                                  | Result                                                    |
| ------------------------------------------------------------------------------------------------------ | --------------------------------------------------------- |
| format                                                                                                 | **fails** (1 file)                                        |
| lint                                                                                                   | **fails** (36 errors, 99 warnings, `--max-warnings 0`)    |
| unit                                                                                                   | **fails**: 917/917 tests pass, line coverage 44.87% < 45% |
| notices, typecheck, rust, rust-coverage, native, contracts, release-versions, smoke, core, performance | pass                                                      |
| `pnpm audit --prod`                                                                                    | no known vulnerabilities                                  |

## P0 — merge gate and developer loop are broken

1. **`pnpm run dev` cannot start.** `tooling/dev.mjs:158` has a top-level `return` inside a
   module-scope `try` (added in `65298e1`). `node --check tooling/dev.mjs` → `SyntaxError: Illegal
return statement`. Exit without `return` (e.g. restructure the block or `process.exit(1)` after
   setting the error).
2. **Format check fails.** `65298e1` moved `openings.json` to `crates/kchess-domain/data/`, but
   `.prettierignore:18` still names `crates/kchess-wasm/js/data/openings.json`. Same stale path in
   `THIRD_PARTY_NOTICES.md:61`. The generated data file should stay ignored, not be reformatted.
3. **Lint fails.**
   - Parse error from item 1.
   - `@typescript-eslint/no-floating-promises`: `tests/core/analysis-engine.test.ts:42,70`.
   - `@typescript-eslint/no-import-type-side-effects`: `apps/desktop/app/stores/local.ts:14`.
   - `logging/no-silent-catch` / `no-silent-promise-catch` (32): all in tests and benchmark tooling
     (`tests/core/*.test.ts` helpers that turn exceptions into `{ error }` values,
     `apps/desktop/tests/**`, `tooling/benchmarks/*.mjs` recorders, `tooling/core-*.mjs`,
     `tooling/guardrail-report.mjs`, `tooling/puzzle-worker-smoke.mjs`). Capturing an error as a
     value is intended there, so collapse each pattern into one shared helper per area
     with one justified `eslint-disable`, rather than one suppression per site. Do not
     weaken the rule for shipped code.
   - 99 `no-non-null-assertion` warnings in shipped code (largest: `apps/desktop/app/utils/themes.ts`
     11, `crates/kchess-wasm/js/variant.ts` 10, `apps/desktop/app/pages/computer.vue` 7,
     `crates/kchess-wasm/js/analysisTree.ts` 6). Replace with real narrowing or explicit errors; do
     not turn the rule off.
4. **Unit coverage is below its threshold** (lines 44.87% vs 45%). Add real tests for uncovered
   pure modules (`crates/kchess-wasm/js/timeControl.ts` 0%, `tvChannels.ts` 0%, `rush.ts` 50%,
   `validate.ts`, `puzzle.ts`), not a lower threshold.

## P1 — correctness and robustness

5. **No cross-process single-instance protection.** `ProfileClaim`
   (`crates/kchess-core/src/facade.rs:177`) is an in-process `HashMap`, and Electron main never calls
   `app.requestSingleInstanceLock()`. A second app launch (common on Windows/Linux) or the CLI on
   the same profile runs a second core over `kchess.db`/`puzzles.db`. That means two Lichess
   event streams for the same account, two engine schedulers, and competing library writes. Fix both layers:
   - Electron: `requestSingleInstanceLock()`; the second instance quits, the first restores and
     focuses its window on `second-instance`.
   - Rust: `ProfileClaim::claim` also holds an OS advisory lock on a lock file in the data dir for
     the core's lifetime (`std::fs::File::try_lock`, or `fs4` if the pinned toolchain lacks it), with
     the existing "already running for this profile" error on contention. Add a regression test
     (second claim from a separate process or file handle fails; release on drop frees it).
6. **Quit can be blocked forever after a renderer crash or hang.**
   `apps/desktop/electron/main/quit.ts` + `index.ts`: if the renderer is gone or unresponsive, each
   flush times out after 3 s and the dialog offers only Retry/Cancel. The app cannot quit except
   by force. Fix: when the renderer process is gone, there is nothing left to flush, so allow quitting.
   Otherwise add an explicit destructive third choice ("Quit without saving") with a clear warning.
   Cover with `QuitPreparation` unit tests.
7. `apps/desktop/electron/main/index.ts:276` logs renderer `warning` console messages through
   `logError`. Map warnings to `logWarn`.

## P2 — architecture and code quality

8. **Clock formatting is duplicated in TypeScript.** `apps/desktop/app/stores/local.ts:88` and
   `stores/kchess/computerGame.ts:110` reimplement the clock text that online play gets from the Rust
   `formatClock` (`@kchess/rules/clock`). That breaks "no chess rules in TypeScript", and the local copies
   never show hours. Use `formatClock` in all three; if its tenths-under-10s output differs, make one
   behaviour the rule (in Rust) and keep all clocks consistent.
9. **"Watch this live game" is copied three times** (`components/FriendCard.vue:31`,
   `components/TournamentEvent.vue:88`, `pages/players.vue:333`). Each writes raw
   `localStorage.setItem('kchess:watch-tab', …)`, and they navigate differently
   (`navigateTo('/watch')` vs `store.selectPage('watch')`). Move it into the watch store as one
   action. Also centralize the duplicated view-preference keys (`kchess:watch-tab`,
   `kchess:practice-tab` in `practice.vue`/`studies.vue`/`useQuickPick.ts`, `kchess:repertoire-study`
   in `OpeningTrainer.vue`/`studies.vue`) as shared constants.
10. `OnlineSession::action` (`crates/kchess-core/src/lichess/online/session.rs:1786`) re-implements
    `live_token` and redundantly writes `current_account` back to itself; reuse `live_token`.
11. `apps/cli/src/index.ts` `ASSISTANCE_METHODS` is a hand-kept copy of the Rust facade's
    assistance gate ("keep in sync"). Generate or export it from `@kchess/contracts` like the arity
    table so it cannot drift.
12. `tooling/eslint/architecture.mjs`: `void SQLITE_EXEMPT` is a dead constant kept alive by a no-op.
    Use it in the rule or delete it.
13. `hosts/node/src/secrets.ts`: support is detected through absolute `secret-tool` paths, but calls
    resolve `secret-tool` through `PATH`. Invoke the detected absolute path.
14. Stale docs: `RUST_MIGRATION.md:208` names the removed `tests/core/store-accounts.test.ts`.
15. CI never runs `cargo deny` although `deny.toml` exists; add it to `ci.yml`.

Process note: the 2026-10-13 bug-audit document records a passing `pnpm run check` that the
committed tree does not reproduce. Run the full check before committing; CI never saw these
commits.

## Fix follow-up (2026-10-14, uncommitted work on `main` above `65298e1`)

All 15 findings fixed in the working tree; `pnpm run check` passes end to end
(format, lint 0 warnings, notices, types, rust, rust-coverage, native, contracts,
release-versions, smoke, core, unit 105 files / 985 tests with lines 45.27% ≥ 45%,
performance). `node --check tooling/dev.mjs` passes. `pnpm run build` passes;
`pnpm run test:e2e` gives 46 passed / 3 failed, and the same 3 fail on a clean
`65298e1` worktree (pre-existing environment issue: the seeded review engine never
produces the Review button on this box, and the two logout failures cascade from its
dirty seed via `UNIQUE constraint failed: accounts.username`).

1. `tooling/dev.mjs`: native-build failure now throws (with `cause`) instead of a
   top-level `return`; outer catch logs and cleans up. Verified `node --check` passes.
2. `.prettierignore` and `THIRD_PARTY_NOTICES.md` point at
   `crates/kchess-domain/data/openings.json`. The generated file stays ignored.
3. Lint green: `tests/core/analysis-engine.test.ts` awaits `stopAnalysis`;
   `stores/local.ts` uses `import type`; silent catches collapsed into one helper per
   area (`tests/core/helpers/capture.ts`, `apps/desktop/tests/unit/capture.ts`,
   `tooling/capture-error.mjs`, each holding the area's single justified disable;
   `tooling/dev.mjs` probes use `attempt`/`attemptAsync`); all 99 `!` replaced with
   narrowing or explicit errors (one fixup: `pageSwipe.ts` needed a `windowOf`
   helper since TS 6 drops const narrowing in closures; one revert: a `fresh`-object
   hoist in `lichessStudies.ts` bypassed Vue reactivity and broke 5 unit tests —
   fixed by reading back through the store).
4. Coverage 44.87% → 45.27% lines via five new suites (`tests/unit/time-control`,
   `tv-channels`, `rush-rules`, `puzzle-rules`, `validate-rules`: 64 tests, 100%
   lines on the five target modules). Thresholds untouched.
5. Single instance, both layers: Electron `requestSingleInstanceLock` (second
   instance quits, first restores/creates its window; startup guarded) and
   `ProfileClaim` holds an OS advisory lock (`profile.lock`, `File::try_lock`,
   unlocked in `release()` so a closed core reopens while its object lives).
   New `crates/kchess-core/tests/profile_lock.rs` (3 tests: foreign-held lock
   refuses claim and `Core::open`; drop releases; close releases and reopen works).
6. Quit trap: `QuitPreparation` returns `true` when the renderer is gone
   (`gone()`, set on `render-process-gone`, cleared on new/finished load) and the
   retry dialog offers Retry / Quit without saving / Cancel (`QuitRetryChoice`).
   `quit.test.ts` updated to the new contract plus 3 new regression tests (10 pass).
   E2E quit-dialog mock updated to the new Cancel index (response 2); both
   `@packaged` quit/close tests pass.
7. Renderer `warning` console messages go through `logWarn` (errors stay `logError`).
8. Tenths under 10 s are now the rule in Rust `format_clock`
   (`0:09.9`, floor-based, clamped); `local.ts`, `computerGame.ts` and
   `local.vue:otbText` use `formatClock`. Golden `training.json` regenerated for the
   5 changed digests only; `native-rules` green incl. `KCHESS_FUZZ_SCALE=20`;
   `tooling/chess-smoke.ts` asserts the new `0:09.` prefix.
9. `watchGame(gameId)` action on the watch store (friends tab + stream +
   `selectPage('watch')`); call sites unified. Shared `utils/storageKeys.ts`
   (`WATCH_TAB_KEY`, `PRACTICE_TAB_KEY`, `REPERTOIRE_STUDY_KEY`) replaces all
   literals. Watch/quick-pick/computer-game/clock tests pass (28 + 38).
10. `OnlineSession::action` uses `live_token(id, "sending an action")`; redundant
    `current_account` write-back removed. Message text unchanged.
11. `ASSISTANCE_METHODS` lives in `kchess_domain::misc::contracts`, is exported by
    `kchess-contracts` into `ts/generated/arity.ts`, and the CLI imports it
    (`@kchess/contracts` added to its deps). `check:contracts` green.
12. `SQLITE_EXEMPT` is now a real exclusion in the `node:sqlite` boundary rule.
13. `secrets.ts` invokes the detected absolute `secret-tool` path
    (`credentialTool`); new Linux unit test asserts every call uses it (6 pass).
14. `RUST_MIGRATION.md` no longer names the removed TypeScript storage suites.
15. `deny.toml` allows the repo-compatible licenses (own GPL-3.0-or-later crates,
    shakmaty, Zlib, CDLA-Permissive-2.0) and path-dependency wildcards;
    `cargo deny check` is fully green and runs as a new CI `deny` job
    (`taiki-e/install-action@cargo-deny`).
