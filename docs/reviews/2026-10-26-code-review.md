# KChess code review — 26 October 2026

Reviewed commit `9d64308ea3f9de7c3e0c0afe02f720dd574b30d0` (package `2026.10.12`). This review identifies **12 actionable findings: one P1 and eleven P2**. Five distinct defects were reproduced with six focused probes; the remaining seven are established by tracing the source and have not been exercised end to end. P1 means prioritize before shipping; P2 means a concrete defect to address next.

This is a broad repository review with deeper inspection of the recent shared-core changes, persistence, engine ownership and frontend synchronization. It does not establish that every bug has been found. No application fixes, dependency changes, commits, pushes, tags or releases were made.

## 1. P1 — Both frontends bypass study conflict protection and silently overwrite other windows

Locations: [Electron studies.ts:129](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/studies.ts:129), [GPUI studies.rs:90](/Users/ketansingrole/git/kchess/apps/gpui/src/studies.rs:90), [core studies.rs:249](/Users/ketansingrole/git/kchess/crates/kchess-core/src/store/studies.rs:249).

The core accepts a third `studyCommand` argument describing the study revision the edit is based on. Both frontend writers pass only the command and `true`. Consequently, the conflict check exercised by the existing service test is never enabled by normal frontend saves. Reloading the library on remote changes does not update the already-open analysis board or its edit baseline.

**Reproduced through two real socket clients:** create e4; another client replaces it with d4; the original client submits e4 e5 with exactly the arguments the frontend sends. The save succeeds and d4 disappears. A separate renderer probe confirms `saveChapter` passes only two arguments.

**Fix:** retain the revision associated with the board's actual content and pass it for every mutation of an existing study. Handle conflicts without retrying the same stale content as an unconditional write. Updating the revision merely because the library refreshed would defeat protection for an older open board. Prefer an explicit monotonic revision over millisecond wall-clock timestamps. Preserve the two-editor sequence as a regression.

## 2. P2 — Computer searches in separate windows cancel each other

Locations: [search.rs:314](/Users/ketansingrole/git/kchess/crates/kchess-core/src/engine/search.rs:314), [engine facade:41](/Users/ketansingrole/git/kchess/crates/kchess-core/src/facade/engine.rs:41), [engineControl.ts:15](/Users/ketansingrole/git/kchess/apps/desktop/app/utils/engineControl.ts:15).

**Source-established:** unlike analysis, computer search still has one core-wide `Search` state. Every new `bestMove` cancels its current token, and `stopEngine` stops that global search. The renderer's ownership ticket only coordinates callers inside its own renderer. If Electron and GPUI each request a computer move, the second request supersedes the first; a takeback or teardown in one app can also stop the other's move. The losing computer game receives a failure instead of its move.

**Fix:** scope search ownership and stop requests to the connection, then arbitrate actual engine resources through the shared scheduler. A global stop remains appropriate for settings changes, assistance admission and core shutdown. Test simultaneous searches and a stop from the other connection.

## 3. P2 — Editing one setting can roll back another window's settings

Locations: [profileSync.client.ts:17](/Users/ketansingrole/git/kchess/apps/desktop/app/plugins/profileSync.client.ts:17), [settingsPersistence.ts:38](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/kchess/settingsPersistence.ts:38), [main index.ts:190](/Users/ketansingrole/git/kchess/apps/desktop/electron/main/index.ts:190).

**Source-established:** the profile plugin does not handle `desktopSettings`. The shell consumes `settings:saved` only to update updater preferences; the renderer receives no corresponding settings refresh. Its next save sends the entire old settings object. For example, window A changes sound volume, then window B changes board appearance: B writes its old sound volume back along with the appearance change. Account-triggered `reloadData()` updates `data.settings` but also leaves the editable `settings` copy unchanged.

**Fix:** synchronize settings to each frontend and persist field patches or enforce a revision with conflict handling. Merge remote changes into untouched fields while preserving local dirty fields. Test two clients changing disjoint fields and the same field.

## 4. P2 — A newly connected window can miss the current notification owner

Locations: [server.rs:357](/Users/ketansingrole/git/kchess/crates/kchess-service/src/server.rs:357), [core.rs:316](/Users/ketansingrole/git/kchess/crates/kchess-core/src/core.rs:316), [Electron fallback:125](/Users/ketansingrole/git/kchess/apps/desktop/electron/main/index.ts:125), [GPUI fallback](/Users/ketansingrole/git/kchess/apps/gpui/src/core.rs).

`core.connect()` reports ownership before the server adds the new client to the host's outlets. If an existing window is focused, ownership does not change at all, so no later ownership event is sent. Both frontends interpret an unknown owner as permission to notify.

**Reproduced:** focus client A, then connect client B and complete a library call. B receives no `alerts:owner` snapshot. A knows it owns alerts while B's default also permits alerts, allowing duplicates until another ownership change reaches B.

**Fix:** include current ownership in the welcome/bootstrap state, or explicitly send a snapshot after registering the outlet. Ensure bootstrap state is available before delivering alert-producing events. Test joining while another window remains focused.

## 5. P2 — Rust calls made after disconnect can wait forever

Locations: [client.rs:164](/Users/ketansingrole/git/kchess/crates/kchess-service/src/client.rs:164), [client.rs:195](/Users/ketansingrole/git/kchess/crates/kchess-service/src/client.rs:195).

The reader drains pending requests once when it closes but records no persistent closed state. The writer's queue can remain open. A later call registers a new oneshot and successfully queues its frame; when writing fails, the writer exits without failing that new pending request. No reader remains to resolve it.

**Reproduced with a scripted local peer:** accept hello, send welcome, close; wait for `ClientEvent::Closed`; call `library`. The call remains pending beyond the probe's 150 ms bound instead of immediately returning a stopped error. GPUI can use this old client while reconnect is in progress.

**Fix:** share a closed state across reader, writer and callers. Either transport task ending must close admission and fail every pending request, with synchronization covering a call racing the drain. Test calls before, during and after EOF and write failure.

## 6. P2 — Closing the Electron service client can still create a live connection afterwards

Locations: [serviceCore.ts:277](/Users/ketansingrole/git/kchess/crates/kchess-node/js/serviceCore.ts:277), [serviceCore.ts:299](/Users/ketansingrole/git/kchess/crates/kchess-node/js/serviceCore.ts:299), [serviceCore.ts:358](/Users/ketansingrole/git/kchess/crates/kchess-node/js/serviceCore.ts:358).

`close()` marks the client closed and ends only `current`. An initial handshake or reconnect is not cancelled; the retry loop never checks `closed`. When an outstanding handshake finishes, it installs `current` even after close. Calls also do not reject based on `closed`.

**Reproduced with a controlled socket:** hold welcome, call `close()`, deliver welcome. `ready` resolves and `connection` becomes 7; the socket was never ended. This can retain a service connection after a host scope is released. A retry already in progress may also continue launching services during shutdown.

**Fix:** use one cancellable connection scope, cancel the socket attempt and retry wait on close, reject later calls, and check the scope before installing any successful connection. Drain pending promises during close.

## 7. P2 — Reconnecting restores transport without restoring frontend activities

Locations: [serviceCore.ts:267](/Users/ketansingrole/git/kchess/crates/kchess-node/js/serviceCore.ts:267), [GPUI reconnect loop](/Users/ketansingrole/git/kchess/apps/gpui/src/core.rs), [core disconnect cleanup:324](/Users/ketansingrole/git/kchess/crates/kchess-core/src/core.rs:324).

**Source-established:** disconnect ends connection-owned analysis, watching and voice. Both clients reconnect with a new connection ID but expose no reconnect generation/bootstrap event to those frontend owners. They neither restart an active analysis/watch nor reconcile the frontend's stale running state. Focus is sent only when a window focus event happens, so the new connection also loses the existing focus state. A recovered socket therefore does not mean the open activity recovered.

**Fix:** report transport lifecycle to the frontend and refresh authoritative state after reconnect. Restart only safe subscriptions and read-only work that the current view still owns; reconcile voice explicitly and never replay an ambiguous move mutation. Test a severed connection while analysis and watching are active, without navigation or a focus change.

## 8. P2 — Backpressure can drop the only final analysis update

Location: [server.rs:94](/Users/ketansingrole/git/kchess/crates/kchess-service/src/server.rs:94).

**Source-established:** the event-name allowlist drops every `engine:analysis` frame when its queue is full, including `done` updates carrying `completed`, `interrupted` or `failed`. Those are terminal state changes; no next update is guaranteed. Dropping the final update can leave a consumer holding a nonterminal analysis indefinitely. The implementation drops incoming frames; it does not retain the latest frame for later delivery despite describing these events as coalesced.

**Fix:** distinguish replaceable progress from terminal events, guarantee terminal delivery or disconnect for resynchronization, and implement a latest-value slot if intermediate frames are genuinely coalesced. Test a full queue when the analysis ends.

## 9. P2 — Cutting off a slow reader can block connection cleanup indefinitely

Location: [server.rs:460](/Users/ketansingrole/git/kchess/crates/kchess-service/src/server.rs:460).

**Source-established:** queue overflow signals the reader loop to stop, but cleanup then awaits the writer before calling `core.disconnect()` and decrementing `active`. A client that keeps its socket open and stops reading can leave `write_frame()` blocked forever. Dropping the sender does not interrupt the pending write. The disconnected client remains a core connection, can retain analysis/watch/voice ownership and alert ownership, and prevents the idle service shutdown.

**Fix:** terminate or bound the writer on cut-off and run core disconnection/accounting regardless of writer completion. Monitor writer failure in the connection select loop as well. Test a connected peer that stops reading while events fill its queue, then verify resource cleanup and idle exit.

## 10. P2 — Profile refreshes can be lost or applied out of order

Locations: [profileSync.client.ts:24](/Users/ketansingrole/git/kchess/apps/desktop/app/plugins/profileSync.client.ts:24), [gameArchive.ts:137](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/gameArchive.ts:137).

**Source-established:** every change event starts an independent library read and applies its reply without a request generation or revision. An older reply arriving later can replace a newer snapshot. Separately, the archive store ignores an incoming snapshot while writing, pending or failed saves exist, with no dirty marker or follow-up refresh. A remote game added during a local save can consequently remain absent after the local save completes, until another event or app restart. The study store similarly discards refreshes while failed intents exist.

**Fix:** coalesce invalidations through one refresh owner, use `RequestScope` or snapshot revisions, and retain a pending invalidation when local writes defer adoption. Refresh after those writes settle. Test reversed replies and a remote change during a local write.

## 11. P2 — The startup deadline does not bound a stalled handshake

Locations: [Electron tryConnect:165](/Users/ketansingrole/git/kchess/crates/kchess-node/js/serviceCore.ts:165), [Rust Client::connect:110](/Users/ketansingrole/git/kchess/crates/kchess-service/src/client.rs:110).

**Source-established:** both startup loops await the entire connection/hello exchange before checking the five-second startup deadline. Neither bounds the read waiting for welcome/refusal. If the service accepts the connection but is stalled before answering, startup can wait indefinitely. The server's timeout only bounds how long it waits for the client's hello; it does not protect the client against a stalled server.

**Fix:** apply a deadline to the whole connect-and-handshake operation and destroy the attempted connection on timeout or cancellation. The outer launch deadline must constrain each nested attempt. Test a peer that reads hello but never answers.

## 12. P2 — Failed preference migration deletes its only recovery copy

Location: [useViewPreference.ts:61](/Users/ketansingrole/git/kchess/apps/desktop/app/composables/useViewPreference.ts:61).

The legacy value is removed before parsing and before the asynchronous core write succeeds. Save failure is logged and consumed, and `flushViewPreferences()` resolves normally. A disk failure or rejected value loses the saved preference on the next launch. `removeItem` is also outside the storage-access try/catch, so a removal failure escapes instead of using the documented fallback.

**Reproduced:** seed a legacy preference, make `setViewPreference` reject with disk unavailable, load and flush. The old localStorage entry is gone and no core write succeeded.

**Fix:** parse first, remove the exact legacy entry only after acknowledged persistence, and retain failed writes for recovery/retry. Guard removal as well as reading. Test write failure followed by reload and subsequent successful migration.

## Verification and retained probes

- The existing service library tests passed: 8 tests. The existing real-socket service suite passed: 5 tests.
- Six focused probes passed while asserting the observed broken behavior: three Rust socket probes, two renderer probes, and one controlled-socket Electron close probe. These are diagnostic reproductions, not passing product regressions.
- The selected existing settings/study/core suites passed: 16 tests. The final full TypeScript unit run passed: **151 files, 1,205 tests**, with diagnostic probes excluded.
- Initial sandboxed socket attempts failed with `EPERM`; the same probes and service tests passed with local socket access. These permission failures are not application findings.
- An initial all-unit run included a temporary audit mock with an import setup error. All 152 other files passed; the mock was repaired and its probe passed independently. The final baseline run excludes all temporary probes.

Retained diagnostic sources:

- [Rust socket probes](/Users/ketansingrole/git/kchess/docs/reviews/2026-10-26-code-review/socket-probes.rs.txt): temporarily copy to `crates/kchess-service/tests/audit_review.rs`, then run `cargo test -p kchess-service --test audit_review --locked`.
- [Renderer probes](/Users/ketansingrole/git/kchess/docs/reviews/2026-10-26-code-review/renderer-probes.test.ts.txt): temporarily copy to `apps/desktop/tests/unit/audit-review.test.ts`, then run that file with Vitest.
- [Electron close probe](/Users/ketansingrole/git/kchess/docs/reviews/2026-10-26-code-review/service-close-probe.test.ts.txt): temporarily copy to `tests/core/audit-service-close.test.ts`, then run that file with Vitest.

Remove the temporary copies afterwards. For implementation, turn each diagnostic into a regression asserting the intended behavior. The `.txt` artifacts deliberately remain outside normal test discovery.

## Remaining coverage and engineering priorities

The main missing practice is failure-sequence testing across actual frontend clients. Existing tests demonstrate the core's optional conflict API, nominal socket sharing and idle shutdown; they do not establish that the frontends use revision protection or recover correctly. Add a two-client suite covering stale document edits, disjoint settings edits, disconnect during work, connection after close, backpressure and deferred refreshes. Keep ownership scopes, cancellation and bootstrap state explicit at each asynchronous boundary.

Also verify the service's engine/runtime/voice asset lifetime in packaged Linux AppImages and during updates: the service binary is copied outside the app, but `ServiceConfig` still points at bundled assets and, for Electron, its runtime executable inside the launching app. This is a platform risk requiring packaging reproduction, not an additional confirmed finding here.

No new production build, Electron E2E run, packaged test, full Rust workspace run, typecheck, coverage run, live Lichess mutation, real microphone session or sustained resource benchmark was performed. No claim of cross-platform completeness or security certification is made. Recommended order: study conflict protection; settings and search ownership; connection close/disconnect/recovery; alert bootstrap and backpressure; ordered profile refresh; preference migration.
