# KChess audit of 30e5503 — 27 October 2026

Reviewed commit `30e5503c8d1891a49260390a03b8bdfa0492f246`, package `2026.10.12`, after the fixes for the three existing October 27 reviews. **Eight actionable P2 findings remain.** Seven have diagnostic reproductions; the GPUI preference finding is established by source tracing. P2 denotes a concrete correctness, persistence, or recovery defect. This review cannot establish that every bug has been found.

The review read the repository guardrails, earlier reports and subsequent fixes, then examined profile synchronization, settings/account integration, study persistence and cloud recovery, GPUI persistence and shutdown, shared-core transport, game/session adapters, engine services and puzzle storage. The deepest verification is in the findings below; passing baseline suites do not establish coverage of all these paths.

## 1. P2 — A stale refresh erases the follow-up refresh it starts

Location: [profileSync.client.ts:54](/Users/ketansingrole/git/kchess/apps/desktop/app/plugins/profileSync.client.ts:54), [library.ts:73](/Users/ketansingrole/git/kchess/apps/desktop/app/utils/library.ts:73).

**Reproduced with the real archive store and native library.** Start a delayed library read containing a game. Another window adds a different game, and this window completes deletion of the original. Deliver the old read. The archive correctly rejects it and synchronously calls `refreshLibraryLater`, which starts a new read and fills the plugin's shared `pending` set. The older callback then executes `pending.clear()`. The new response finds no pending stores, so it reads the correct library but updates nothing. In the reproducer, the core contains the other window's game while the renderer stays empty after both reads finish.

The current stale-read tests assert that a deleted game stays hidden or a renamed study stays renamed. Those local states are already correct before the follow-up read, so they pass even when that read is discarded.

**Fix:** consume a per-read copy of the pending stores before calling store callbacks; preserve any stores queued by a reentrant refresh. After each potentially reentrant callback, avoid continuing an obsolete generation. Keep a regression that requires an additional remote change to appear after the replacement read.

## 2. P2 — A successful incremental study command is treated as failed and replayed

Location: [studies.ts:170](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/studies.ts:170), [studies.ts:189](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/studies.ts:189).

**Reproduced with the real study store and native library.** This window knows study A. Another window creates B before its profile refresh is adopted. This window duplicates A. The core commits the duplicate and returns an incremental result containing the new duplicate plus an order that includes B. This window has no body for B, so reconstructing the order throws after the mutation succeeded. The catch treats this display/adoption error as a failed command and queues it for replay. Strict quit flush duplicates A again, throws again, and still blocks normal quit. The diagnostic shows three persisted studies after the first duplication and four after one quit retry.

This also prevents simple refresh recovery: the store defers snapshot adoption while its failed-command queue is nonempty.

**Fix:** distinguish durable command success from local snapshot adoption. An unknown ID should trigger a full reload without replaying the acknowledged command. Use stable operation identities for operations that create documents if ambiguous transport results are to be retried. Test remote creation before duplication and before chapter creation, including quit retry and successful recovery without reopening.

## 3. P2 — Account-operation responses turn other windows' settings into local edits

Location: [kchess.ts:625](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/kchess.ts:625), [kchess.ts:665](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/kchess.ts:665). Similar direct assignments appear after adding/importing/connecting accounts and clearing synced data.

**Reproduced in the real renderer store.** This window's form and saved base have volume 0.7. Another window saves volume 0.8. Removing a followed account returns authoritative `AppData` containing 0.8. The account handler assigns the entire result to `data` but leaves the form at 0.7. Change an unrelated setting such as zen mode and save: the persistence owner now compares the unchanged form volume 0.7 against the new base 0.8, treats it as a deliberate edit, and sends 0.7 back. The diagnostic observes the stored renderer settings reverting after that save.

`reloadData` uses `adoptStored`, but account-operation results bypass it. The recent read-generation fix does not cover these responses.

**Fix:** adopt every incoming `AppData` through one mutation-aware owner that rebases untouched form fields before advancing the saved settings base. Also order overlapping account-operation responses against each other, rather than applying generations only to `reloadData`. Preserve the account-operation → unrelated settings edit regression.

## 4. P2 — Chapter autosave overwrites another window's completed study rename

Location: [studies.ts:224](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/studies.ts:224), [core studies.rs:323](/Users/ketansingrole/git/kchess/crates/kchess-core/src/store/studies.rs:323).

**Reproduced with the real study store and native library.** Open “Original” in window A and track its chapter. Window B renames it to “Remote rename”. Before A receives the profile update, A autosaves another move. `saveChapter` sends A's stale study name along with the PGN; the core's `save` updates both. The PGN conflict check passes because B changed no moves, and the durable name becomes “Original” again.

This is distinct from the previously fixed stale library-read rollback: there is no stale library response in this reproducer. An unrelated field is overwritten by the content mutation itself.

**Fix:** make a chapter-content save update only chapter content. Resolve the study's current name in the core, or give renaming a separate explicit intent that content autosave never performs. Test rename in B followed by autosave in A before delivery of profile notifications.

## 5. P2 — Autosave resurrects a remotely deleted study with a different chapter ID

Location: [core studies.rs:274](/Users/ketansingrole/git/kchess/crates/kchess-core/src/store/studies.rs:274), [core studies.rs:328](/Users/ketansingrole/git/kchess/crates/kchess-core/src/store/studies.rs:328).

**Reproduced with the real study store and native library.** A has an open study and a `basedOn` chapter PGN. B deletes the study. A autosaves before adopting B's deletion. `check_based_on` treats a missing study/chapter as valid; `save` then creates a study under the old study ID and generates a new chapter ID. A still refers to the old chapter ID. The next autosave is rejected with “That chapter cannot be saved.” The remote deletion is undone and the open board can no longer continue saving to its chapter.

**Fix:** when a caller supplies a conflict base or existing chapter ID, require that target to exist. Return a recoverable deleted-target conflict and retain/export local work rather than recreating the document implicitly. Keep explicit creation and restore as separate operations. Test delete in B → autosave in A → another move, including recovery without losing A's local PGN.

## 6. P2 — Cloud upload recovery does not track moves and tags separately

Location: [Lichess studies.rs:330](/Users/ketansingrole/git/kchess/crates/kchess-core/src/lichess/studies.rs:330), [Lichess studies.rs:370](/Users/ketansingrole/git/kchess/crates/kchess-core/src/lichess/studies.rs:370), [Lichess studies.rs:448](/Users/ketansingrole/git/kchess/crates/kchess-core/src/lichess/studies.rs:448).

**Two retry states reproduced against a deterministic local HTTP fixture.** Updating a chapter sends moves and tags in separate requests. If moves land but the tags request fails, the remote chapter has local movetext and baseline tags. When an existing tag was edited, retry matches neither the complete baseline nor the complete desired chapter and refuses it as a cloud conflict, even though it is the app's own interrupted upload.

When an optional tag was removed, the opposite occurs. `uploaded` compares only tags present in the desired chapter; it never checks that removed baseline tags are absent. The diagnostic removes `Annotator`, supplies the moves-only intermediate remote state, and observes successful sync with no POST requests and `Annotator` still present in the returned chapter. The caller can then mark this incomplete upload as the new cloud baseline.

The existing partial-upload regression covers a completed first chapter followed by an untouched second chapter; it does not cover interruption between the two requests for one chapter.

**Fix:** reconcile movetext and tags as separate phases, including intended tag deletions. Accept only known baseline/desired states for each phase and perform the remaining phase without automatically replaying ambiguous move mutations. Add both edited-tag and deleted-tag interrupted-upload cases.

## 7. P2 — Export reports success when Lichess imports no chapters

Location: [Lichess studies.rs:286](/Users/ketansingrole/git/kchess/crates/kchess-core/src/lichess/studies.rs:286), [LichessStudies.vue:172](/Users/ketansingrole/git/kchess/apps/desktop/app/components/LichessStudies.vue:172).

**Reproduced against a local HTTP fixture; the real API behavior is documented upstream.** `export_to_lichess_study` discards the parsed import response and returns the study ID for any successful HTTP status. A fixture returning HTTP 200 with an empty `chapters` array is accepted as success. The renderer then says the chapter was added.

This is a real boundary case: Lichess maintainers confirm that a full 64-chapter study can return 200 while creating no chapter, and that a multi-game import can succeed only partially; the body identifies what was imported. See [Lichess issue #17737](https://github.com/lichess-org/lila/issues/17737). A successful transport status is insufficient acknowledgment of the requested export.

**Fix:** validate the returned imported-chapter list against the requested games and report zero/partial import accurately. Preserve local documents and avoid marking a complete cloud baseline for missing chapters. Add full-study and partially accepted multi-game fixtures. The local probe tests response handling, not a live Lichess upload.

## 8. P2 — GPUI drops failed view-preference writes and has no quit flush for them

Location: [prefs.rs:50](/Users/ketansingrole/git/kchess/apps/gpui/src/prefs.rs:50), [prefs.rs:75](/Users/ketansingrole/git/kchess/apps/gpui/src/prefs.rs:75).

**Source-established; no GPUI disk-failure UI run.** `set` updates the in-memory value before sending the write, then only logs rejection. It retains no failed intent and registers no quit barrier. After a rejected write, selecting the same value returns early because the optimistic cache already contains it; a recovered service still receives no retry. `forget` also removes the cached key first and silently discards its result, so a failed deletion has no retry path either. Normal quit can report successful preparation even though these changes were refused, and restart restores the old preference.

The Electron preference owner now retains failed writes and retries at quit; this corresponding GPUI path remains unhandled.

**Fix:** use one serialized preference writer with durable acknowledgments, retain the latest failed intent per key, and register its strict flush with the quit gate. Clear an intent only after acknowledgment or supersession. Test rejected set/delete → service recovery → same-value selection or quit → restart.

## Verification and retained reproductions

- `pnpm exec vitest run`: **153 files passed, 1,228 tests passed**, with one file/test failing because its local HTTP listener was denied by the sandbox (`listen EPERM`). Rerunning the complete affected `tests/core/store-wrappers.test.ts` with loopback access passed **all 10 tests**. This was not a single unrestricted full run and did not collect coverage.
- `cargo test --workspace --locked` with local fixture permissions: **771 tests passed, zero failed**, across 57 test/doc-test summaries. The initial sandboxed attempt stopped at 16 managed-engine fixture tests denied socket access; those are not reported as product failures.
- `pnpm run typecheck`: **passed**.
- Renderer diagnostics: **five passed**, reproducing findings 1–5 with the real stores and native library fixture.
- Core diagnostics: **three passed**, reproducing the two states in finding 6 and the response-handling defect in finding 7 using local HTTP servers. No live account mutation occurred.
- Application implementation files were not changed. Existing untracked reports were preserved. No commit, push, tag or release was created.
- No production build, full check/coverage/lint run, desktop E2E, packaged test, interactive GPUI run, cross-platform installer execution or dependency advisory audit was performed. No claim is made about current dependency vulnerabilities or complete UI/accessibility coverage.

The diagnostic sources are retained outside active suites because they assert the observed defects, not intended behavior:

- [Renderer probes](/Users/ketansingrole/git/kchess/docs/reviews/2026-10-27-latest-audit/renderer-probes.test.ts.txt): copy temporarily to `apps/desktop/tests/unit/audit-latest-2026-10-27.test.ts`; run `pnpm exec vitest run apps/desktop/tests/unit/audit-latest-2026-10-27.test.ts`; remove the temporary copy.
- [Core probes](/Users/ketansingrole/git/kchess/docs/reviews/2026-10-27-latest-audit/core-probes.rs.txt): copy temporarily to `crates/kchess-core/tests/audit_latest_2026_10_27.rs`; run `cargo test -p kchess-core --test audit_latest_2026_10_27 --locked` with local socket permission; remove the temporary copy.

## Engineering practices to address

The concrete gaps are reentrancy-safe refresh ownership, distinguishing committed writes from response-adoption errors, one settings-rebase path for every incoming snapshot, content commands that change only their owned fields, conflict handling for deleted targets, recovery across individual remote mutation phases, validation of domain acknowledgments, and persistence barriers shared by both frontends. Turn the retained sequences into intended-behavior regressions as fixes land. Prioritize study write/replay correctness and the settings overwrite, followed by refresh recovery, cloud acknowledgment/recovery and GPUI preferences.
