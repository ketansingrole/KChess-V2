# KChess code review — 16 October 2026

Reviewed `ef08115` (`2026.10.8`). Application source was not changed. This review found **eight reproduced bugs: two P1 and six P2**, followed by two performance improvements. P1 means prioritize before shipping the affected behavior; P2 means a concrete defect to schedule next. Passing existing tests does not cover the sequences below.

## 1. P1 — Quit can overwrite a newer study with an obsolete failed save

Location: [studies.ts:24](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/studies.ts:24), particularly failure matching at line 30 and retry at line 53.

Failed commands are retained for quit-time retry. A later command only supersedes a failure if their complete JSON matches, including the PGN. A successful save with newer content consequently leaves the older failed save in the retry list.

**Reproduced against the real Rust library:** save e4; fail an update to d4; successfully update the same study to c4; flush for quit. The database contains c4 before the flush and d4 afterwards. The successful quit flush silently rolls back the user's latest saved work. A stale command can also refer to a chapter or study subsequently removed, so replay must account for later mutations, not just successful edits.

**Fix:** track intent/revision per study and chapter. Supersede obsolete failed writes when a newer mutation succeeds, and invalidate failures made irrelevant by deletion. Retry only the latest still-applicable intent. Preserve this exact sequence as a regression that expects c4 after quit.

## 2. P1 — A delayed “Save as study” reply links and overwrites a different document

Location: [analysis.ts:218](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/analysis.ts:218).

`saveAsStudy()` captures a PGN and awaits creation, then unconditionally assigns the returned study ID, chapter ID and saved PGN to the current analysis board. It does not check whether the user loaded another document while the request was pending. Other navigation operations already use `studyNavigation`, but this operation does not.

**Reproduced:** begin saving e4 as “Original”; hold the creation reply; load d4; deliver the e4 save result; flush analysis. The d4 board is now linked to “Original” and replaces its e4 content in the database.

**Fix:** capture a document/navigation generation before awaiting and check it before linking the result. Keep the successfully created study in the library even if the board has changed. Distinguish edits to the same document from replacement with another document. The regression should preserve e4 in “Original” and leave d4 unlinked.

## 3. P2 — Autosave can undo a successful study rename

Location: [analysis.ts:261](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/analysis.ts:261), with the queued save at line 269.

The content autosave includes `open.name`, captured before its queued command reaches the core. An in-flight rename has not updated the renderer's library yet. Editing the board during that request queues a content save carrying the old name behind the rename.

**Reproduced:** create “Old name”; hold the reply to renaming it “New name”; edit a comment and flush; complete the rename. The final persisted study has the new comment but is named “Old name”. This can occur when returning to Analysis before a rename request finishes.

**Fix:** make a chapter-content update independent of study metadata. Alternatively, resolve unchanged metadata from authoritative state when executing the command and enforce a revision check. The regression should retain both “New name” and the comment.

## 4. P2 — A fresh computer game stalls after its engine turn is cancelled for online play

Location: [computerGame.ts:63](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/kchess/computerGame.ts:63).

When assistance becomes available again, the watcher restarts the computer only if `saved` was truthy when the store was constructed. That variable describes startup restoration; it never becomes truthy for a game started in the current fresh session. `ComputerGame.availabilityChanged()` cancels work and updates permission but does not itself restart a turn.

**Reproduced:** with no restored computer session, play e4; interrupt the pending engine turn by disabling assistance; settle the cancelled reply; restore assistance. No second search is requested, the game remains at e4, and the human cannot move because it is Black's turn. In timed games the resumed computer clock can continue without a search.

**Fix:** attempt `computerTurn()` whenever the engine becomes ready and assistance is permitted. Its existing state checks decide whether a turn is due. Add the fresh-session cancellation/recovery sequence, alongside restored-session coverage.

## 5. P2 — A late knight-drill summary destroys an active challenge

Location: [KnightPath.vue:67](/Users/ketansingrole/git/kchess/apps/desktop/app/components/KnightPath.vue:67).

The mounted hook waits for the saved-score summary, then resets the board to `e4 → e4`, with an optimal length of zero. Start is usable during that read.

**Reproduced in the mounted component:** start before `runSummary()` resolves, then complete the summary read. The running challenge becomes “Reach e4 in 0 jumps”, with both the knight and target on e4. It is impossible to earn a shortest-path point for that challenge. The run clock is already counting down.

**Fix:** initialize the idle board synchronously before awaiting, or only apply that initial layout while the same idle generation is current. Loading a score summary should never reset a game in progress.

## 6. P2 — A completed knight round's timer can replace the next round

Location: [KnightPath.vue:126](/Users/ketansingrole/git/kchess/apps/desktop/app/components/KnightPath.vue:126).

The 450 ms round-advance timeout is neither stored nor tied to its originating round. Skip, finish and restart do not invalidate it. Its sole guard is that some run is currently running.

**Reproduced:** reach the goal; press Skip during the success delay; make one jump in the new challenge; let the old timeout fire. The next challenge is replaced and its jump count resets to zero. Ending and quickly restarting a run has the same generation problem. The completed goal also remains interactive during the success delay.

**Fix:** retain one timeout and invalidate it on Skip, finish, restart and unmount. Capture the run and round generation, and disable further jumps after reaching the target until the next round starts. Verify that a delayed callback cannot modify a later challenge.

## 7. P2 — Late summary reads install keyboard listeners after unmount

Locations: [SquareColorTrainer.vue:42](/Users/ketansingrole/git/kchess/apps/desktop/app/components/SquareColorTrainer.vue:42) and [CoordinatesTrainer.vue:81](/Users/ketansingrole/git/kchess/apps/desktop/app/components/CoordinatesTrainer.vue:81).

Both mounted hooks await `runSummary()` before adding the global keydown listener. Leaving the drill while the read is pending runs cleanup before registration. Completing the read then installs a listener belonging to an unmounted component, retaining its state. If it was running when the user left, later keyboard input can still mutate that detached session. A rejected summary also prevents keyboard controls from being installed in the first place.

**Reproduced for Square Color:** mount with a deferred summary; unmount; resolve it. A new global keydown listener is registered after unmount. Coordinates has the same ordering in its source.

**Fix:** register keyboard controls synchronously and load optional scores separately with error handling and a current-owner check. Alternatively, check the mounted generation immediately after the await before registering anything. Preserve deferred-success-after-unmount and summary-failure cases.

## 8. P2 — The repertoire trainer ignores every chapter after the first

Location: [OpeningTrainer.vue:28](/Users/ketansingrole/git/kchess/apps/desktop/app/components/OpeningTrainer.vue:28).

The trainer parses `study.pgn`, which is the first chapter's mirror. The saved study supports multiple chapters, but this screen has no chapter selector or mechanism to train the remaining chapters. “Drill as repertoire” from the library therefore exposes only a subset of the selected study.

**Reproduced:** save a repertoire with an e4 chapter and a d4 chapter, select the study for training, and play d4. The trainer rejects it as “Not in your repertoire. Expected e4.” The d4 chapter cannot be selected from the trainer.

**Fix:** reuse the study chapter selection UI and train the selected chapter, or explicitly rotate across chapters. Keep independently rooted chapter positions; concatenating their PGNs into one line would be incorrect. Include chapter identity in persisted drill selection and any chapter-specific learning state.

## Performance improvements

### Bound the renderer's full-review cache

[review.ts:26](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/review.ts:26) retains every streamed or loaded full review in a Map. It has no eviction policy, and `dispose()` only unsubscribes; it does not release reviews, summary records or stream timestamps. Background review and browsing different games can grow this cache throughout the app session, including data no longer visible. Full evaluation arrays are much larger than history summaries.

Use a bounded LRU for full reviews, pin the currently displayed review, and retain inexpensive summaries separately within an explicit budget. SQLite already provides authoritative storage. Verify cache size and retained bytes over a long background-review/browsing session. This is a code-established retention improvement; no process heap or RSS measurement was taken, so this review does not claim an observed out-of-memory failure.

### Avoid replaying the full game for clock presentation and expiry

[local.rs:112](/Users/ketansingrole/git/kchess/crates/kchess-domain/src/games/local.rs:112) computes the result by reconstructing the position and checking history. `remaining_at()` calls `running()` and then `turn()`, which repeat reconstruction. `expire_at()` calls those paths again. The renderer's [local.ts:92](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/local.ts:92) runs a 100 ms ticker; the computer clock has equivalent repeated queries. The JSON boundary also serializes the move list for each call.

A focused benchmark executed the shipped WebAssembly binding in Node, with one expiry call and two remaining-time calls per cycle. Warm runs took approximately **0.13 ms at 10 plies, 0.26–0.36 ms at 40 plies, and 0.54–0.66 ms at 160 plies**. These measurements exclude Vue proxies, rendering and the additional calls needed to request host reads, and are not browser frame timings. They show avoidable scaling, but do not establish a current frame-budget failure.

Cache position, turn and terminal/draw state by setup/move revision inside the rules owner, and expose one compact clock projection for both sides. Preserve history for repetition and refresh that derived state when moves change; ticking the display does not require recomputing it. Treat this as a lower-priority efficiency improvement behind the correctness fixes.

The existing library pagination budget remains healthy: a 100-row fixture page was **30,401 bytes**, queried in **0.11–0.15 ms** across databases containing 1,000, 5,000 and 50,000 synthetic games. That benchmark tests its existing fixture and does not bound arbitrary user PGN sizes.

## Verification and reproduction artifacts

- Existing TypeScript unit suite: **115 files, 1,033 tests passed** (`pnpm exec vitest run`).
- Rust workspace: **450 tests passed**, plus successful doc-test runs (`cargo test --workspace --locked`).
- Ten focused diagnostic probes passed: eight distinct findings above, one supporting failure/recovery probe for PGN replacement, and the clock benchmark. Study probes use the real Rust storage backend; component probes stub visual controls and defer IPC replies.
- The initial sandboxed test attempts could not bind loopback fixtures (`EPERM`). Both complete suites passed after rerunning with local fixture access. Those initial failures are environmental, not reported application defects.
- The library structural performance check passed (`node tooling/library-benchmark.mjs --check`).
- Full application typecheck passed (`pnpm run typecheck`); ESLint passed with zero warnings (`pnpm run lint`). The diagnostic test file also passed the test TypeScript configuration after correcting its review-only type import.

[Diagnostic source](/Users/ketansingrole/git/kchess/docs/reviews/2026-10-16-code-review/repro.test.ts.txt) and [probe output](/Users/ketansingrole/git/kchess/docs/reviews/2026-10-16-code-review/probe-output.txt) are retained. The probes deliberately assert the observed broken behavior; passing them demonstrates the findings, not correct product behavior. To replay, copy the source to `apps/desktop/tests/unit/audit-2026-10-16.test.ts`, run `pnpm exec vitest run apps/desktop/tests/unit/audit-2026-10-16.test.ts`, then remove that temporary test. For fixes, convert each probe into a normal regression that asserts the intended behavior.

## Scope and limits

The review traced representative persistence, document ownership, online recovery and assistance admission, engine scheduling/cancellation, IPC/origin protection, local/worker host lifecycle, puzzle/store access, review retention, update handling and training flows. It also checked earlier review reports against the current implementation. No new confirmed security-boundary flaw was found in the inspected paths; this is not a security certification or a claim that every bug has been found.

No application fix, dependency change, release operation, commit or push was performed. No new production build, coverage run, Electron E2E run, installer verification, live Lichess mutation, microphone session or update installation was performed. Cross-platform behavior and sustained CPU/memory use remain outside the exercised scope. Recommended implementation order: eliminate both content-overwrite paths, separate autosave metadata, restore cancelled computer turns, then fix training lifecycle and chapter coverage before optimizing the measured paths.
