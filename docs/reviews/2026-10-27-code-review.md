# KChess code review — 27 October 2026

Reviewed `aef8bc4` (package `2026.10.12`). Found **10 actionable issues: two P1 and eight P2**. P1 means address before shipping the affected feature; P2 means a concrete defect to fix next. Four diagnostic probes reproduced observed behavior (three renderer probes and one extracted updater-helper probe). The other findings are supported by source traces, with their runtime verification limits stated below.

This review inventories the repository and goes deeper on the shared-core follow-up, frontend persistence/recovery, and new GPUI updater. It does not establish that every bug has been found. Yesterday's audit targets an older commit; `7f5986f` addresses its findings, so this report does not simply repeat that list. No application fixes, dependency edits, commits, pushes, tags or releases were made.

## 1. P1 — The Linux updater trusts a predictable shared temporary directory

Location: [updates.rs:628](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:628), [download:319](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:319), [apply:510](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:510).

**Source-established security defect; no cross-user exploit run.** Downloads use `temp_dir()/kchess-update/PREFIX/version`. On Linux's normal shared `/tmp`, another local user can precreate the `kchess-update` parent with permissions allowing KChess to create its children. The updater neither checks ownership/symlinks nor creates a private parent. The parent owner can replace the staged pathname after checksum verification. Linux's `signer` returns an empty signer, and `apply` later trusts that pathname without verifying the bytes again: an AppImage becomes executable, or a substituted .deb is handed to privileged `dpkg`.

The same directory also causes ordinary multi-instance corruption: a second GPUI process, including a development build, unconditionally removes the first one's verified staged download in `init` through `tidy(None)`. Separate profiles and app installations still share the prefix. Concurrent downloads also remove each other's version directory.

**Reproduced lifecycle-helper collision:** use the exact `downloads`, `tidy`, `aside` and `incoming` functions extracted from the current source; write instance A's staged file; run the startup cleanup that instance B runs; A's file is deleted. This probe does not claim to reproduce the cross-user attack.

**Fix and regression:** use an exclusively created, user-private workspace with ownership/symlink checks, retain ownership until install finishes, and coordinate installations with a lock. Validate the final artifact through a stable file/owned staging location before privileged installation. Test two live instances, development startup beside a downloaded update, and a precreated foreign-owned staging parent.

## 2. P1 — Install-on-quit starts the Windows installer before document flushes finish

Locations: [updater quit hook:690](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:690), [quit collection:343](/Users/ketansingrole/git/kchess/apps/gpui/src/core.rs:343), [analysis flush:440](/Users/ketansingrole/git/kchess/apps/gpui/src/pages/analysis/mod.rs:440).

**Source-established; not exercised on Windows.** `core::quit` first invokes every registered flush callback to collect futures, then awaits them. The updater callback performs `apply(..., false)` synchronously during collection. It is registered during application initialization, before the analysis page registers its session/study flush. On Windows, `apply` immediately spawns the NSIS installer with `--updated /S`. The installed NSIS template allows a short exit interval, then stops the running application and can force-kill it. Therefore slow settings/study writes can still be pending, and analysis flush callbacks may not even have run, when installation starts terminating the app.

The local `app-builder-lib` NSIS template confirms the stop/force-kill sequence in `templates/nsis/include/allowOnlyOneInstallerInstance.nsh`, `_CHECK_APP_RUNNING`. On Linux .deb installations the same synchronous callback waits for `pkexec/dpkg` on the UI thread while quitting.

**Fix and regression:** make update application a distinct phase after successful persistence flushes, with a detached installer/helper that waits for the old process to exit. Keep package-manager waits off the UI thread. Hold a document save beyond the installer's shutdown interval and assert installation cannot start before acknowledgment.

## 3. P2 — An edit during a settings save can undo another window's unrelated setting

Locations: [Electron settingsPersistence.ts:45](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/kchess/settingsPersistence.ts:45), [GPUI save result:199](/Users/ketansingrole/git/kchess/apps/gpui/src/settings.rs:199).

**Reproduced in the Electron persistence composable.** Start with this window's sound volume at 0.2 and another window's persisted volume at 0.8. Save an appearance change; the core correctly returns the merged settings with volume 0.8. Before that response arrives, edit zen mode. The response advances `data.settings` to the merged copy, but the changed form prevents adoption of any returned fields. The next save now treats the untouched form volume 0.2 as an intentional change from its new base 0.8, and writes it back. The GPUI result handler similarly replaces `stored` but preserves the entire form when another edit has occurred.

**Fix and regression:** rebase the current form onto each acknowledged result, preserving only fields changed since the sent snapshot. Track local changes by field rather than treating the entire form as dirty. Preserve this three-party sequence, including an edit while the first save is pending.

## 4. P2 — A refresh during deletion leaves a successfully deleted game visible

Locations: [gameArchive.ts:129](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/gameArchive.ts:129), [refresh guard:148](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/gameArchive.ts:148).

**Reproduced.** `remove` optimistically removes the row and awaits a serialized mutation. `replaceFromLibrary` defers for writes, pending saves and failures, but does not check the `mutations` counter. A previously read library response containing that game can therefore restore it while deletion is pending. When deletion succeeds, its completion removes only the `saved` entry; the row remains visible. This reproduces a UI resurrection, not a proven reinsertion into the database.

**Fix and regression:** defer snapshot adoption during every mutation, then refresh when deletion settles; alternatively apply deletion tombstones/revisions to incoming snapshots. Test a snapshot read before deletion and delivered during its pending response.

## 5. P2 — Reconnection does not refresh missed profile changes

Locations: [Electron profileSync.client.ts:25](/Users/ketansingrole/git/kchess/apps/desktop/app/plugins/profileSync.client.ts:25), [GPUI profile subscription:356](/Users/ketansingrole/git/kchess/apps/gpui/src/core.rs:356).

**Electron registration gap reproduced; GPUI source-established.** Both transports emit `core:reconnected`, but profile synchronization listens only for future `profile:changed` events. Neither the reconnect handshake nor this listener fetches authoritative library/accounts/settings snapshots. A game, study or account changed while the client was disconnected remains missing/stale after reconnect if nothing else changes. The retained probe verifies that invoking transport recovery triggers no library read from the profile plugin.

**Fix and regression:** reconnect through a bootstrap/resynchronization owner that refreshes profile snapshots before announcing recovery complete, while retaining local pending writes. Disconnect one client, change documents/settings from another, reconnect without another mutation and verify convergence.

## 6. P2 — A restarted core does not resume the open online game

Locations: [Electron startup-only resume:839](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/kchess.ts:839), [GPUI startup-only resume:955](/Users/ketansingrole/git/kchess/apps/gpui/src/online.rs:955), [transport recovery:334](/Users/ketansingrole/git/kchess/crates/kchess-node/js/serviceCore.ts:334).

**Source-established; no live Lichess mutation performed.** Analysis, watching and voice handle `core:reconnected`; the online controllers do not. `resumeOnline` runs during frontend initialization. If the service itself exits and reconnect starts a fresh core, that core has no attached board stream, while the frontend retains its previous game and clocks. Transport recovery alone does not rerun the safe ongoing-game lookup/attachment. User moves can fail against a core with no current game, and the open board stops receiving authoritative game events until an explicit recovery action.

**Fix and regression:** explicitly mark the online connection unverified and resume from authoritative Lichess state after core recovery. Never replay a pending move mutation. Use a local stream fixture to restart the service during a game and verify reattachment, clock reconciliation and pending-move ambiguity handling.

## 7. P2 — GPUI still applies asynchronous profile refreshes out of order

Locations: [settings reload:113](/Users/ketansingrole/git/kchess/apps/gpui/src/settings.rs:113), [study reload:335](/Users/ketansingrole/git/kchess/apps/gpui/src/studies.rs:335), [account refresh:112](/Users/ketansingrole/git/kchess/apps/gpui/src/accounts.rs:112).

**Source-established; no GPUI UI scheduling probe.** These refreshes spawn independent reads and apply every result, without generations or snapshot revisions. A read of older state delayed behind a newer read can become the final visible settings/studies/accounts. The settings reload additionally awaits custom themes before applying its settings response, widening the stale-result window. An old study reload can also replace the result of a more recent local command. Yesterday's `RequestScope` fix protects Electron profile refreshes, but these GPUI owners still lack its equivalent.

**Fix and regression:** use one refresh owner per surface with generation/revision checks and account for intervening local mutations. Test reversed responses and a local successful save between a read and its application.

## 8. P2 — A stalled update transfer can leave downloading stuck indefinitely

Locations: [HTTP client:261](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:261), [checksum request:301](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:301), [asset stream:328](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:328).

**Source-established; no stalled-server runtime probe.** The client sets a connect timeout, and the release lookup adds a request timeout. Checksum and installer downloads add neither a total deadline nor a read/idle timeout. A server that accepts the connection and stops sending headers or body can hold the worker forever. The UI remains `Downloading`, disables further checks and offers no cancellation/retry route. Dropping a GPUI task does not cancel `on_runtime`'s independent thread/runtime.

**Fix and regression:** bound idle reads and checksum retrieval, choose an appropriate transfer policy for large packages, and wire an explicit cancellation token through requests and worker ownership. Test a peer that stalls after accepting and a peer that stops halfway through the body.

## 9. P2 — Installation has no exclusive state and can run concurrently

Locations: [Updates::install:867](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:867), [replacement:557](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:557).

**Source-established; destructive installation race not run.** `install` clones the same staged path and starts an independent background runtime without changing phase or claiming an installation lock. The restart action remains available during a slow installation. A repeated invocation can run two replacements/installers; quitting during explicit installation can also invoke `apply` again because `staged` is cleared only on completion. Replacing `self.work` does not stop the first runtime thread. On macOS both operations manipulate the same installed path and `.app-old` backup, including deleting that backup at the start of replacement.

**Fix and regression:** introduce an `Installing` phase and an exclusive apply operation shared by explicit restart and quit. Take ownership of the staged artifact before starting work. Test repeated requests and quit while installation is pending, asserting one installer/replacement invocation.

## 10. P2 — macOS replacement is not recoverable across a crash during cross-volume copy

Locations: [replace:557](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:557), [copy_tree:590](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:590), [startup cleanup:641](/Users/ketansingrole/git/kchess/apps/gpui/src/updates.rs:641).

**Source-established; no real installation interrupted.** Replacement first renames the working app to `.app-old`, then moves the staged app into place. If staging and installation are on different filesystems, it falls back to copying directly into the final app path. Process termination or power loss during that copy leaves an incomplete bundle at the normal launch path; the existing rollback runs only when the function returns an error. There is no next-launch recovery protocol, and `tidy` deletes `.app-old` rather than validating the replacement and recovering it. A crash between the first two renames also leaves the normal app path absent.

**Fix and regression:** copy into a sibling staging location on the destination filesystem and validate it before touching the working bundle; retain a recoverable transaction/backup until successful launch. Recover interrupted states before cleanup. Test interruptions after moving the old bundle and halfway through copying.

## Verification

- Existing targeted renderer/client suites: **5 files, 15 tests passed**.
- Full TypeScript unit run, before diagnostic files were added: **153 files passed; 1,213 tests passed**, with one test failing because the sandbox denied its local HTTP listener (`EPERM`). Rerunning that entire affected file with local socket permission passed **all 10 tests**. Thus every baseline test passed across these runs, but there was no single unrestricted full-suite run or coverage run.
- Existing service tests, rerun with local socket permission: **9 integration tests and 10 library tests passed**. The sandboxed attempt failed on socket binding, not a demonstrated product failure.
- Existing GPUI updater tests: **8 passed**, including checksum rejection and macOS signer checks.
- `pnpm run typecheck`: **passed**.
- `pnpm run check:fast`: exited successfully with **no changed files selected**; this is not additional regression evidence. The retained Markdown report passed a separate Prettier check.
- Retained renderer diagnostic probes: **3 passed asserting the observed broken behavior**, after correcting a missing event-registration mock in the third probe. The first attempt had two passing probes and that test-harness error.
- Extracted updater staging helper probe: **passed asserting the observed deletion**.
- No production build, Electron E2E, packaged upgrade run, full Rust workspace/coverage run, Windows/Linux installation execution, cross-user exploit, live Lichess action, real microphone session or dependency-advisory audit was performed.

## Retained reproductions and missing engineering practices

[Renderer diagnostic source](/Users/ketansingrole/git/kchess/docs/reviews/2026-10-27-code-review/renderer-probes.test.ts.txt): temporarily copy to `apps/desktop/tests/unit/audit-2026-10-27.test.ts` and run `pnpm exec vitest run apps/desktop/tests/unit/audit-2026-10-27.test.ts`. Remove the temporary copy afterwards. Its assertions pin current defects; implementation should turn them into regressions asserting the intended behavior.

[Updater staging diagnostic source](/Users/ketansingrole/git/kchess/docs/reviews/2026-10-27-code-review/updater-staging-probe.rs.txt): compile a temporary `.rs` copy with `rustc --edition=2024`, then run it. It uses the current source's cleanup/path helpers with minimal type scaffolding and an audit-only prefix; it does not install or replace an application.

The concrete practices missing here are field-level rebasing after save acknowledgments, mutation-aware snapshot adoption, reconnect bootstrap, generation checks in GPUI, private updater staging, a single installation owner, flush-before-install ordering, bounded/cancellable network work, and recoverable filesystem transactions. Tests currently exercise individual helpers and nominal recovery more strongly than these cross-owner failure sequences. Add those specific sequences instead of broad implementation-mirroring tests.

Prioritize secure staging and quit ordering, then persistence/reconnect correctness, then installer exclusivity, deadlines and interruption recovery. Treat real Windows, macOS cross-volume and Linux packaged-upgrade fixtures as necessary follow-up validation for the updater.
