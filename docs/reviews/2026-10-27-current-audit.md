# KChess audit of current code

Reviewed `18d8a0c8272e4bb7e5e6f80456191649fa8a5f65` (2026.10.12), after the fixes covered by the two pre-existing October 27 reports. Seven actionable findings remain: one P1 and six P2. P1 means a data-loss path to prioritize before shipping; P2 means a concrete correctness or recovery defect. This review does not establish that every issue has been found.

The review covered repository guardrails, recent fixes, frontend persistence and synchronization, shared-service request lifetime, and GPUI update installation. Three renderer sequences were reproduced. The GPUI findings below are established by source tracing, without interactive window-close testing or Windows/Linux installer execution.

## 1. P1 — Closing the GPUI window bypasses the save gate and destroys its save owners

Locations: [menu.rs:106](/Users/ketansingrole/git/kchess/apps/gpui/src/menu.rs:106), [pages/mod.rs:63](/Users/ketansingrole/git/kchess/apps/gpui/src/pages/mod.rs:63), [analysis/mod.rs:440](/Users/ketansingrole/git/kchess/apps/gpui/src/pages/analysis/mod.rs:440), [analysis/mod.rs:870](/Users/ketansingrole/git/kchess/apps/gpui/src/pages/analysis/mod.rs:870).

The Quit menu now calls `prepare_quit`, but Close Window directly calls `remove_window`. The workspace owns the page entities; the navigator and page quit callbacks hold weak references. Removing the window drops those page owners before the subsequent non-macOS `on_window_closed` callback calls `cx.quit()`. The registered page flush can then return `None` at `weak.upgrade()?`.

Concrete sequence: edit a study or analysis position and immediately close the window before its 150 ms debounce fires. The delayed task is owned by the page and the quit callback no longer has a page to flush. The latest moves can disappear. On macOS, closing the window leaves the app running and does not run the app quit barrier at all; reopening reconstructs pages from the older saved session. This also bypasses the new retry/discard/cancel prompt for page-owned saves.

Fix: intercept every window close while the workspace still exists, flush its documents with the same failure handling as Quit, and remove the window only after acknowledgment or explicit discard. Include native close controls and the Close Window action. Retain a regression for edit → immediate close → reopen, plus a rejected save during close on each platform.

## 2. P2 — An old library snapshot can persistently undo a completed study rename

Locations: [profileSync.client.ts:38](/Users/ketansingrole/git/kchess/apps/desktop/app/plugins/profileSync.client.ts:38), [studies.ts:284](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/studies.ts:284), [studies.ts:219](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/studies.ts:219).

Reproduced in the real study store with the native library fixture: create “Original”; capture a library snapshot; successfully rename to “Renamed”; deliver the earlier snapshot; then autosave another move. `replaceFromLibrary` checks failed writes but has no mutation revision. It restores “Original” in the renderer. `saveChapter` builds its next command using that stale name, and the core stores “Original” again. This goes beyond a temporary display inconsistency.

The latest plugin change captures a mutation revision only for played games. Studies, mistake drills, repertoire misses and joined tournaments receive unversioned snapshots. Their corresponding stale-read sequences need coverage too; the persistent rename rollback is the reproduced case here.

Fix: track mutations for each library owner and reject/reload snapshots that predate them, including responses arriving after a mutation completes. Preserve chapter conflict bases separately from library freshness. Test rename/autosave, study creation/deletion, drill answers and joining/leaving while a library read is pending.

## 3. P2 — Electron account reads can restore an account after local removal

Locations: [kchess.ts:789](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/kchess.ts:789), [kchess.ts:605](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/kchess.ts:605).

Reproduced in the real renderer store: begin a delayed `reloadData`; remove a tracked account and adopt the successful result; deliver the old data. The removed account returns to `data.accounts`. `reloadData` orders reads against other reads and settings writes, but account mutations do not invalidate it. The durable account removal remains correct; the renderer is stale and can offer actions for a removed account until a later refresh.

Fix: invalidate pending account/data reads when local account mutations start and commit, and reread when necessary. Include connect, sync, clear-data and add/remove operations in the ownership policy. Test a read delivered after a completed removal, rather than only reversed read/read responses. GPUI's account generation fix does not cover Electron.

## 4. P2 — Windows releases the installation lock while its installer is still running

Locations: [updates.rs:873](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:873), [updates.rs:513](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:513).

`apply_on_quit` acquires an `InstallLock`, then returns `apply(...)` for every non-Deb package. Windows `apply` only spawns NSIS; it does not wait for installation to finish. Returning drops the lock immediately, while the child installer is still working. Another GPUI process can acquire the same lock and launch another installation into the same directory. The existing exclusivity test proves two held locks exclude each other, but does not exercise the real spawn-and-return path.

Fix: give the installer/helper ownership of the installation lock until completion, or retain an independent supervising process that waits for the installer. Do not rely on the app remaining alive after it has initiated quit. Add a two-process fixture with a deliberately blocked installer and verify the second attempt remains excluded until that child exits.

## 5. P2 — The Deb helper still has a lock handoff gap before cleanup

Locations: [updates.rs:900](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:900), [updates.rs:907](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:907), [updates.rs:862](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:862).

The package now survives workspace cleanup, but ownership is reacquired rather than transferred. The parent retains the installation lock until process exit. The shell helper opens the lock separately and waits in `flock`. After parent exit, a newly launched app can acquire the lock before the helper does; `tidy` then deletes the installation's `.deb`. The helper subsequently proceeds with a missing package. An existing waiting locker is not a durable ownership handoff. If `flock` is absent or fails, the script also continues without establishing ownership.

Fix: pass an inherited owned lock to a dedicated helper with a readiness acknowledgment, and require successful lock ownership before installation or cleanup. Keep the artifact through authorization and package-manager completion. Test immediate relaunch with the helper paused before acquisition, as well as missing/failed lock support. The current helper-artifact test holds a lock manually and does not test this handoff.

## 6. P2 — Failed ordinary preference writes are discarded and quit reports success

Locations: [useViewPreference.ts:99](/Users/ketansingrole/git/kchess/apps/desktop/app/composables/useViewPreference.ts:99), [library.client.ts:18](/Users/ketansingrole/git/kchess/apps/desktop/app/plugins/library.client.ts:18).

Reproduced: change a preference; reject `setViewPreference` with disk full; wait for that request to settle; call `flushViewPreferences`. The flush resolves successfully and does not retry. `save` consumes the rejection and removes the entry from `pending`, leaving no failed intent for shutdown. Normal edits have no legacy recovery entry, so restarting restores the old preference even though quit acknowledged completion. The earlier migration fix correctly retains legacy text; it does not retain ordinary failed edits.

Fix: retain the latest failed value per key, retry it at flush, and report persistent failures to the existing quit handshake. A subsequent successful edit should supersede the failed intent. Test failure → quit retry → service recovery, and failed deletion of account-specific preferences during logout.

## 7. P2 — GPUI save preparation has no timeout or way to cancel a stalled save

Locations: [core.rs:414](/Users/ketansingrole/git/kchess/apps/gpui/src/core.rs:414), [core.rs:347](/Users/ketansingrole/git/kchess/apps/gpui/src/core.rs:347), [client.rs:250](/Users/ketansingrole/git/kchess/crates/kchess-service/src/client.rs:250).

`prepare_quit` waits for `all_saved` without a deadline. The retry/discard/cancel prompt is reached only after a save returns an error. An established service connection that remains open but stops answering a save leaves the future pending indefinitely; the client's handshake deadline does not bound calls. `Quitting.preparing` remains true, so another Quit request cannot initiate recovery. Restart-and-install uses the same path and remains Installing. The later five-second core-close timeout cannot help because execution has not reached core close.

Fix: bound save preparation and treat an unanswered flush as a failure with the same retry/discard/cancel options, preserving uncertain writes and avoiding overlapping retries. Electron's `QuitPreparation` provides an existing behavior to match. Test a peer that receives a save and never replies, without closing its socket.

## Verification and retained evidence

- Baseline `pnpm exec vitest run`: 153 files passed; 1,222 tests passed. One test timed out after its local HTTP listener was rejected with `EPERM` by the sandbox. The complete affected `store-wrappers.test.ts` file passed all 10 tests when rerun with loopback permission. There was no single unrestricted full-suite run and no coverage run.
- `cargo test -p kchess-gpui --bin KChess updates::tests --locked`: all 15 updater tests passed, with loopback permission for local HTTP fixtures.
- `pnpm run typecheck`: passed. An initial run caught a readonly assignment in the temporary diagnostic harness; the harness was corrected to initialize through `reloadData`, then typecheck passed.
- Three diagnostic renderer probes passed, asserting the observed broken behavior in findings 2, 3 and 6. They are retained in [renderer-probes.test.ts.txt](/Users/ketansingrole/git/kchess/docs/reviews/2026-10-27-current-audit/renderer-probes.test.ts.txt). To rerun, copy temporarily to `apps/desktop/tests/unit/audit-current-2026-10-27.test.ts`, run that file with Vitest, and remove the temporary copy. Convert these to intended-behavior regressions when implementing fixes.
- No implementation files changed. The pre-existing untracked audit reports were preserved. No commit, push, tag or release was created.
- No production build, interactive GPUI close/reopen test, Electron E2E, Windows/Linux installer execution, full Rust workspace test/coverage run, packaged upgrade or dependency advisory audit was performed.

The recurring engineering gaps are incomplete mutation revisions across owners, save ownership tied to destructible views, failures consumed before shutdown can see them, unbounded save barriers, and installer locks whose lifetime stops before the work they protect. Prioritize the window-close data-loss path, then study/account ordering, then installer ownership and persistence recovery. Avoid broad refactoring or extra boilerplate tests until these concrete sequences have regressions.
