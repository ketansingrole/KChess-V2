# KChess code and performance review — 5 October 2026

Reviewed commit `27753fc69f6eb59ddf6617242e0772ca4103cd92`, package version `2026.10.3`. Application source was left unchanged. Temporary diagnostic tests used isolated stores, an in-memory SQLite database and mocked network calls; they were removed after the review.

## Implementation follow-up — all eight findings fixed

The findings below describe the reviewed commit. All eight have now been addressed in the working tree:

- Cloud output is invalidated when assistance becomes unavailable, including while Analysis is detached. The page also gates cloud scores, PVs, playable suggestions and arrows. Mounted-page and late-response regressions cover live play and remounting.
- Online recovery retains the protected game's owning account and requires an authoritative server response before releasing protection. Missing credentials emit auth-required; removed accounts and cancelled recovery attempts have regressions.
- A conflicting restored analysis document becomes an unsaved recovery copy. Study edits are flushed before switching documents and before unload; failures of either storage write preserve the previous persisted document and surface an error.
- Migration 20 adds and backfills game-to-review associations. Updates retain every association, including stronger cached Lichess reviews and identical queued searches. History, Insights and the background queue use the mapping.
- Watch generations reject delayed tour metadata and stale stream acknowledgments after stop, unmount or another selection. Early stream events are buffered until their session ID arrives.
- GIF export uses a dedicated worker with one transferred RGBA frame outstanding at a time, per-frame palettes, progress and cancellation. It no longer retains the whole game's uncompressed frames. Compressed output and chess positions still grow with game length; this is not a measurement of total process memory.
- Analysis persistence debounces serialization and caches the PGN between edits. A legal 9,330-node branching study regression verifies that 20 forward/start navigation cycles cause zero PGN serializations, while consecutive edits serialize once after the debounce.
- TV/game feeds emit typed session-scoped connection status. Failed or ended feeds freeze the displayed clocks and offer Retry; cancellation and stale status events are ignored.

Final validation: `npm run check` passed (formatting, lint, types, smoke tests and **317 unit tests**); `npm run build` passed; **19 production Electron E2E tests passed in 38.5 seconds**. The new 401-frame export test cancels without saving, retries successfully, and verifies animation callbacks continue during encoding. Its initial run exposed a moving Cancel button, corrected by giving the progress counter a fixed width. The final large-study test was also run directly and checked with ESLint and the test TypeScript configuration after expanding its fixture.

No package dependencies or release versions changed. Validation used the isolated branded macOS Electron launcher with fresh profiles, not newly built installers. The original platform and live-service validation limitations below still apply.

The current architecture has useful foundations: authenticated top-level IPC, main-owned credentials and network access, bounded UCI lifecycle handling, a shared search scheduler, a single puzzle database worker, transactional puzzle replacement, paged game history and PGN annotation preservation. Those fixes should be retained. The remaining priority is closing assistance and persistence gaps, followed by renderer work that still blocks interaction.

There are **eight actionable findings: three P1 and five P2**. P1 means address before shipping the affected behavior; P2 means schedule next. “Reproduced” below means an isolated probe demonstrated the state transition, not that every scenario was exercised in the Electron UI. “Code finding” means the call path establishes the issue but its failure scenario was not run.

## Findings

### 1. P1 — Cached cloud analysis remains visible during a live human game

**Reproduced at store level; display path confirmed in code.** [askCloud](/Users/ketansingrole/git/kchess/app/stores/analysis.ts:348) clears its result only when the FEN changes. When assistance becomes disallowed at the same FEN, it cancels future requests but retains the existing cloud result. The [analysis page](/Users/ketansingrole/git/kchess/app/pages/analysis.vue:80) selects those cloud lines without checking `assistanceAllowed`.

With the local engine switched off, an existing cloud evaluation continues to drive the score, principal variations and move arrows when `onlinePhase` becomes playing, seeking or disconnected. Navigating back to Analysis during the game also retains this result. Main's rejection of new cloud requests does not hide previously fetched assistance.

The probe loaded cloud lines for the analysis position, switched off the local engine, attached analysis and changed online phase to playing. Local evaluation became null and `assistanceAllowed` became false, but cloud lines remained eligible for the page's `usingCloud` branch.

**Fix:** gate every displayed assistance source with `assistanceAllowed`, and clear cloud output when assistance is disabled. Invalidate outstanding requests on that transition. Add a mounted-page regression that checks score, PV list, arrows and “play best” controls after a live game begins and after leaving/reopening Analysis.

### 2. P1 — Reconnect can remove live-game protection without checking the server

**Reproduced.** [OnlineSession.resume](/Users/ketansingrole/git/kchess/src/main/lichess.ts:1114) silently skips connected accounts whose token cannot be retrieved. If no request reports a game or throws, [the final branch](/Users/ketansingrole/git/kchess/src/main/lichess.ts:1135) clears `protectedGame` and emits idle, even if the account owning the protected game was never queried.

The probe resumed a known rapid game, cancelled its connection, then made credential retrieval return null. A subsequent resume issued no server request, reported no game and changed `session.playing` from true to false. Both main's assistance guards and the renderer can therefore permit assistance while that human game may still be running.

This is distinct from a server response confirming no ongoing game. Missing credentials can result from unavailable OS credential encryption or an unreadable token.

**Fix:** retain protection until the owning account returns an authoritative no-game response. Treat unavailable credentials as auth-required. Validate the resume attempt before every terminal state mutation, including paths that skip accounts. Test missing/decryption-failed credentials, account removal and superseded resume attempts.

### 3. P1 — A restored analysis session can overwrite a newer saved study

**Reproduced with conflicting persisted documents.** The analysis session restores its PGN and study ID independently from the study library. [Study setup](/Users/ketansingrole/git/kchess/app/stores/analysis.ts:170) takes the last saved PGN from the library, while the editable tree comes from the separate analysis session. [Autosave](/Users/ketansingrole/git/kchess/app/stores/analysis.ts:193) then saves that tree over the linked study after an edit, without comparing revisions.

The two localStorage documents have independent debounced writes and error handling. An interrupted write, storage failure or restored backup can leave them inconsistent. In the probe, the library contained `1. d4`, while the older analysis session contained `1. e4` linked to the same study. Adding a comment to the restored board replaced the library's d4 document with e4.

**Fix:** establish one authoritative study document and persist navigation separately. If session edits must be recoverable, store a base revision/content hash and preserve a conflicting recovery copy rather than overwriting the library. Test interruption between the two writes and failure of either storage write.

### 4. P2 — Identical move sequences lose their association with earlier games

**Reproduced.** Reviews are keyed by root FEN and move sequence, but [writeReview](/Users/ketansingrole/git/kchess/src/main/reviewStore.ts:44) stores only one `gameId`. Saving another game with the same moves replaces that association. History summaries, Insights joins and background review eligibility all use this single game ID.

The probe wrote a complete review for Game0001 and then the same move sequence for Game0002. Only Game0002 retained its summary, and `gamesToReview` returned Game0001 as unreviewed. The background queue can repeatedly relink the shared review between games, leaving some games without accuracy despite already having suitable engine output. Short resignations and repeated opening lines make this practical without requiring a hash collision.

**Fix:** separate shared evaluation data from game-to-review associations using an append-only migration. Preserve a mapping for every game. Test two different game IDs with identical lines, both participants' account copies and a stronger existing review being linked to another game.

### 5. P2 — A late broadcast lookup starts downloading after leaving Watch

**Reproduced.** [openTour](/Users/ketansingrole/git/kchess/app/stores/watch.ts:93) awaits tour metadata and unconditionally starts the chosen round afterwards. [Leaving Watch](/Users/ketansingrole/git/kchess/app/pages/watch.vue:29) stops the stream and unsubscribes, but does not invalidate that metadata request.

The probe began a deferred tour lookup, called `stop()` and `unlisten()`, then delivered the response. It called `watchBroadcast`, restored the round ID and registered listeners again. A hidden stream can therefore continue downloading after the user leaves, switches tabs or closes the tour. A late response for an older tour can also replace the newer selection.

**Fix:** increment a request generation in stop, close and each new selection; reject stale metadata and session replies before changing state or opening a stream. Invalidate all pending watch work when the route detaches. Test a delayed tour response after route exit and rapid tour switching.

### 6. P2 — GIF export blocks the renderer and retains all uncompressed frames

**Measured CPU cost; memory derived from allocations.** [gameGif](/Users/ketansingrole/git/kchess/app/utils/gameImage.ts:141) draws every position, stores a fresh RGBA buffer for each and invokes [encodeGif](/Users/ketansingrole/git/kchess/app/utils/gif.ts:165) synchronously. After images load, neither drawing nor encoding yields to the renderer event loop. Declaring the entrypoint async does not move this work off the UI thread.

At the default 360×416 canvas size, each frame occupies 599,040 bytes. A 60-move game with 121 positions retains **69.1 MiB** of RGBA buffers; the permitted 600-frame limit retains **342.8 MiB**, before canvases, positions, encoder allocations and output copies.

A synthetic 121-frame encoder probe took **348–408 ms synchronously** across diagnostic runs. It reused one RGBA buffer to isolate encoder CPU cost; those runs did not measure the real export's peak memory or canvas painting. Actual game drawing adds further work, and image content changes encoding cost. Even the isolated encoder pauses interaction for substantially longer than one frame.

**Fix:** move encoding to a worker and transfer buffers, process frames incrementally with a fixed/bounded palette strategy, and support cancellation/progress. Avoid retaining the whole game's RGBA data. Test a long export while making board moves and navigating, and measure peak renderer memory as well as duration.

### 7. P2 — Navigation serializes the entire analysis tree before the save debounce

**Measured.** [The persistence source](/Users/ketansingrole/git/kchess/app/stores/analysis.ts:87) calls `treeToPgn(root)` alongside path/orientation. [persistSession](/Users/ketansingrole/git/kchess/app/utils/sessionPersistence.ts:33) watches that source directly. Every path or orientation change executes full PGN generation to evaluate the watcher; the 150 ms debounce applies only to the later save. Saving executes the source again.

A legal 9,330-node tree, within the import limit, produced a roughly 94 KB PGN. Plain-tree serialization took approximately **1.3–2.4 ms**, but alternating start/forward navigation through the real reactive analysis store and flushing its watchers took **10–19 ms per step**. This probe excludes browser rendering and board layout. It demonstrates avoidable work near or above a 60 Hz frame budget on a large supported study; it is not evidence that ordinary small games have this latency.

**Fix:** cache the PGN by document edits, and let path/orientation saves reuse it. Debounce expensive serialization itself rather than just localStorage writes. Avoid reparsing the full PGN on each study edit when the existing validated tree can supply the document. Verify navigation on large branching studies and repeated wheel/keyboard input.

### 8. P2 — Dropped TV/game feeds leave stale boards with running clocks

**Code finding.** [Spectator.watch](/Users/ketansingrole/git/kchess/src/main/spectate.ts:108) catches stream errors and sends no error/ended state to the renderer. Its initial IPC call already returned a session ID, so the store's invocation catch cannot observe a later HTTP failure or stream stall. The last frame remains unfinished, and [the Watch clock](/Users/ketansingrole/git/kchess/app/pages/watch.vue:40) continues interpolating from its timestamp.

After a dropped connection, a board can appear live while its clock counts down to zero from stale data. An initial feed failure can leave an empty board without a useful error. Broadcast round failures have a separate update mechanism; TV and individual games need equivalent lifecycle reporting.

**Fix:** emit a typed per-session connected/reconnecting/ended/error update, freeze clocks when the feed is unavailable and expose retry. Add deterministic fixtures for failure before the first frame, disconnect after moves, idle timeout and cancellation. Reject status updates for older sessions.

## Validation and performance evidence

| Check                                                 | Result                                                         |
| ----------------------------------------------------- | -------------------------------------------------------------- |
| Formatting, ESLint, renderer/Electron/test typechecks | Passed                                                         |
| Chess, puzzle and main smoke tests                    | Passed with local loopback access                              |
| Existing unit suite                                   | 30 files, 293 tests passed                                     |
| Production build                                      | Passed; renderer payload verifier reported 7.5 MiB             |
| Production Electron tests                             | 18 passed in 34.3 seconds, isolated profiles                   |
| Focused behavior/CPU probes                           | Findings above reproduced or measured; temporary tests removed |

The initial combined `npm run check` was stopped by the sandbox denying a fixture's `127.0.0.1` listener. Formatting, lint and types had already passed. `npm test` subsequently passed with loopback access. This was an environment restriction, not an application test failure.

The existing library benchmark confirmed why pagination should remain:

| Stored games | Full list SQL | Full list clone | Full list serialized size | 100-row SQL |
| ------------ | ------------: | --------------: | ------------------------: | ----------: |
| 1,000        |       1.35 ms |         0.98 ms |                    304 KB |     0.09 ms |
| 5,000        |       5.21 ms |         5.50 ms |                   1.52 MB |     0.13 ms |
| 50,000       |      56.97 ms |        56.03 ms |                   15.2 MB |     0.14 ms |

These are local, in-memory structural measurements using synthetic rows. The full-list path is a comparison in the benchmark, not the current history UI's request path. A page transferred approximately 30.4 KB at all three sizes. This review found no reason to undo paging or prioritize a framework rewrite.

## Scope and remaining validation

The review traced engine startup/search/cancellation, scheduler ownership, privileged IPC/origin/CSP boundaries, Lichess request admission and game recovery, SQLite review/library storage, puzzle worker/import ownership, persisted renderer documents, analysis/watch stores, voice lifecycle and export work. Existing update, game, puzzle, review, voice and packaged-protocol regression coverage was exercised through the suites above.

This was a local macOS review. It did not test Windows/Linux installers, a live Lichess account, a real update installation, real microphone speakers, sustained battery use or an extended production CPU/memory trace. No new dependency-advisory audit or registry update comparison was performed; the existing advisory report was not reclassified. Production E2E tests used a fresh build rather than newly built installers.

A useful follow-up is a production trace covering a large study, a long GIF export, background review and sleep/resume during an online game. Windows also needs a targeted check of managed-engine replacement while a warm native process is open: the install handler does not stop engine owners before replacing the executable. That platform-specific failure was not reproduced here and is not counted among the eight findings.

Recommended implementation order: close both assistance gaps; reconcile restored study documents; preserve review-to-game associations; then fix Watch lifecycle and move expensive export/persistence work off the interaction path. Each change should add regressions for its concrete trigger before broadening features.
