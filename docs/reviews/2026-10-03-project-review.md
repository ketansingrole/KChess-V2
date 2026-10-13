# KChess code and technology review

Reviewed 3 October 2026, against commit `1ff8fad` and package version `2026.10.3`.

**Recommendation:** keep Vue, Nuxt, Nuxt UI, Electron, chessops and Chessground. Improve resource ownership, data preservation and recovery first. Trial TypeScript 6, apply compatible maintenance updates, and make stronger engines optional downloads. A framework rewrite has no demonstrated benefit for the problems found here.

This is the original review and adoption plan. Subsequent application and dependency changes are recorded in [the implementation report](2026-10-03-implementation.md). Priorities below are my assessment: P1 means address before substantial new features; P2 means schedule next; P3 means an improvement or experiment. “Reproduced” means an isolated probe demonstrated the behavior. “Code finding” means the implementation supports the finding, but that failure scenario was not exercised in the UI.

## Project understanding and validation

KChess is an offline-capable desktop chess application with local computer play, Lichess Board API play, account/game synchronization, game review, a variation-based analysis board, a board editor, puzzles, practice drills and local English voice recognition.

The renderer is a Nuxt client SPA with hash routing and Pinia state. Electron main owns authentication, networking, SQLite, downloads, updates and child engines. A typed preload bridge connects them. Packaged pages use the `kchess://app` protocol. Stockfish runs outside the renderer; Vosk already uses a worker and loads on demand.

Strengths worth retaining:

- Sandboxed renderer, context isolation, no renderer Node integration, typed IPC methods and Valibot validation.
- OAuth PKCE and state checking, encrypted tokens with rejection of Linux's plaintext storage fallback, verified engine/model downloads and bounded diagnostics.
- SQLite WAL and ordered migrations, cached profiles, incremental game sync and on-demand PGN retrieval.
- Shared rules and chess primitives, board cleanup, stale computer-search guards, voice-result invalidation and microphone cleanup.
- A supervised branded development launcher, installer payload checks, cross-platform CI, smoke tests, mounted-store/component tests and real Electron tests.

Checks performed:

| Check                                                 | Result                                                          |
| ----------------------------------------------------- | --------------------------------------------------------------- |
| Prettier, ESLint, renderer/Electron/test typechecking | Passed on the existing dependencies                             |
| Chess, puzzles and main-process smoke tests           | Passed                                                          |
| Vitest                                                | 18 files, 200 tests passed                                      |
| Production build                                      | Passed; renderer payload verifier reported 8.2 MiB              |
| Electron UI tests against the fresh build             | 11 passed in 24.2 seconds, using isolated profiles              |
| TypeScript 6.0.3 trial                                | Electron, generated renderer and Vue test-project checks passed |
| Full dependency audit                                 | 21 affected package entries: 19 high, 2 moderate, 0 critical    |
| Production-only dependency audit                      | 0 reported advisories                                           |

The initial combined `npm run check` stopped because the sandbox denied a smoke-test loopback server. The smoke and unit tests subsequently passed with loopback access. TypeScript 6 was downloaded into a temporary folder and passed explicitly to the existing checker; this was not a full installed-dependency migration or a lint/build test under a changed lockfile.

Limits: this review did not run Windows/Linux locally, test an actual signed installer/update, play against the live Lichess service, evaluate speech with real speakers, or collect a sustained production performance trace. CI configuration is present, but remote CI status was not verified. Passing tests do not establish an error-free application.

## Findings to fix first

### 1. P1 — An unusable puzzle replacement deletes the previous database

**Reproduced.** [storeSample](/Users/ketansingrole/git/kchess/src/main/puzzleQueries.ts:32) deletes the old puzzles, skips invalid rows, and commits even when zero rows survived. [installPuzzleDb](/Users/ketansingrole/git/kchess/src/main/puzzleDb.ts:57) detects the empty result only after that commit.

An in-memory database containing one valid puzzle became an uninstalled, zero-puzzle database after replacement with an invalid puzzle. This contradicts the intended behavior of retaining the old copy on failure. It affects downloadable cache data rather than accounts or scores, but recovering requires another large download.

**Change:** validate a replacement before swapping it, or throw on zero valid inserted rows inside the transaction before commit. Preserve the previous metadata too. Count actual inserts if duplicates are accepted. Check cancellation immediately before replacement. Add regression cases for empty/all-invalid replacements and an interrupted import retaining the previous sample.

### 2. P1 — PGN import/export silently loses information

**Reproduced.** [treeFromPgn](/Users/ketansingrole/git/kchess/app/utils/analysisTree.ts:137) retains moves and variations but discards game headers, comments and NAG annotations. [treeToPgn](/Users/ketansingrole/git/kchess/app/utils/analysisTree.ts:120) creates a fresh game and copies only SAN. The store calls it without the original headers.

A probe with Alice/Bob, result `1-0`, an event, a clock comment, prose comment and `$1` exported with unknown names/event, result `*`, and no annotations. Valid moves survived. Importing a study to inspect it should not strip its content on export.

**Change:** represent a document separately from its selected position: headers/result at document level and comments, NAGs and clock/evaluation annotations on nodes. Reuse chessops' PGN data rather than building another parser. Report skipped illegal moves and multiple-game input instead of silently accepting only part of the document. Test semantic round trips, including variations, nonstandard starts, Unicode comments and black-to-move positions.

### 3. P1 — Engine startup and stop can wait forever

**Code finding.** Computer play has a 15-second watchdog. [analysis](/Users/ketansingrole/git/kchess/src/main/analysis.ts:104) waits for `uciok`, `readyok` and the old search's `bestmove` without host deadlines. [review](/Users/ketansingrole/git/kchess/src/main/review.ts:151) also waits indefinitely for initialization, readiness and a search result.

`go ... movetime ...` asks the engine to stop; it cannot recover a hung engine. A selected executable that stays alive without speaking UCI can leave analysis busy or hold up the review queue. A review cancellation only sets a flag checked between searches.

**Change:** create one shared UCI process controller with bounded startup/readiness/stop waits, search deadlines, error propagation and termination escalation. Stop the active search on cancellation. Use separate logic for intentionally infinite searches: watch initialization and stop acknowledgment, not an arbitrary total runtime. Preserve partial review results and expose an actionable retry state. Test silent startup, ignored stop, broken pipes, exit during handshake and cancellation.

### 4. P2 — Interrupted analysis is cached as completed analysis

**Code finding.** In [analysis.ts](/Users/ketansingrole/git/kchess/src/main/analysis.ts:120), any `bestmove` with no pending replacement calls `flush(true)`, including a user stop. The [renderer cache](/Users/ketansingrole/git/kchess/app/stores/analysis.ts:181) skips further analysis when a cached entry is `done` and has enough lines.

Leaving the page after shallow output can therefore make a later visit reuse the partial search as finished. Cache entries also contain no engine identity, and the watcher does not track the configured engine. Switching to stronger native Stockfish can continue showing the old engine's cached result for a position.

**Change:** separate termination from quality: completed, interrupted, failed and superseded reasons, plus achieved depth/nodes/budget and engine identity. Reuse partial evaluations immediately while continuing an appropriate new search. Refresh after engine changes. Add a regression for leaving and returning during a shallow search.

### 5. P2 — Connection failure does not have a corresponding game state

**Code finding.** [OnlineSession.listen](/Users/ketansingrole/git/kchess/src/main/lichess.ts:704) reports a string and returns after a permanent failure or exhausted retries. The renderer subscribes that error to the generic toast handler; its phases are only idle, seeking, playing and finished.

A failed asynchronous seek can leave “looking for an opponent” active after the seek has ended. A terminated game stream can leave an apparently live board and interpolating clocks. The latency displayed by presence polling measures a separate HTTP request, not the health of the authoritative game stream.

**Change:** send typed connecting/reconnecting/disconnected/auth-required states, session/game IDs, last authoritative update and retry information. Make recovery a visible action in the game panel. Bind the playing account to the session rather than the mutable account preference. Give ordinary HTTP operations deadlines and streams a heartbeat/stall policy. Reconcile moves with the server after ambiguous acknowledgments instead of blindly retrying a mutation.

Tests should cover a rejected seek, token revocation, sleep/resume, interrupted streams, a move accepted before the response is lost and account switching during a game. Keep these tests deterministic with local protocol fixtures.

### 6. P2 — Presence polls overlap and can restore stale state

**Code finding.** [pollPresence](/Users/ketansingrole/git/kchess/app/stores/kchess/onlineGame.ts:148) is called by a fixed interval and multiple watchers without an in-flight guard or generation check. A slow request can overlap the next poll. A response arriving after the game finishes can repopulate state the phase watcher cleared.

The comment describes a five-second interval while the constant is three seconds; do not rely on that comment as a verified API limit.

**Change:** allow one request in flight, schedule the next poll after completion, and reject responses from an earlier session/account. Deduplicate status requests from Friends and Online. Add a shared rate-limit policy for ordinary Lichess requests, with foreground priority, cancellation and a cooldown on 429. Existing loops serialize their own work; they do not impose global serialization across independent callers. Long-lived game streams need their own lane. Check the [official API documentation](https://lichess.org/api) when implementing endpoint-specific rules.

### 7. P2 — Bulk puzzle work blocks Electron main

**Measured structural issue.** [storeSample](/Users/ketansingrole/git/kchess/src/main/puzzleQueries.ts:32) validates and inserts the entire sample synchronously. Sampling also parses and processes CSV in main, progress calls rebuild the deduplicated sample, and deleting puzzles runs synchronous `VACUUM`.

One local probe used 100,000 synthetic valid puzzles in an in-memory SQLite database:

| Operation                         |      Elapsed |
| --------------------------------- | -----------: |
| Validation and replacement import |       539 ms |
| 50 status reads                   | 367 ms total |
| 60-puzzle rating ladder           |        17 ms |

These are single-run diagnostic measurements, not product benchmarks. The synthetic rows share a simple position; disk I/O, real puzzles and other hardware can differ. They demonstrate synchronous occupancy of the caller's thread. They also suggest optimizing every ladder query is less urgent than moving bulk import and avoiding unnecessary status scans.

**Change:** put sampling/validation and bulk database work in a worker or database service. Cache size metadata at import time; `requireInstalled` only needs the metadata row, not a table-wide byte sum. Use inexpensive progress counters and periodic snapshots. Ensure a clear database writer/transaction owner; do not share a `DatabaseSync` object across threads or introduce competing writers accidentally.

Electron explicitly recommends moving CPU-heavy work off main. Its [performance guide](https://www.electronjs.org/docs/latest/tutorial/performance) and [utility-process API](https://www.electronjs.org/docs/latest/api/utility-process) provide the relevant mechanisms. A worker is a reasonable first step; a utility process adds stronger failure isolation.

### 8. P2 — Security boundaries need origin/frame checks and a production CSP

**Hardening finding; no exploit demonstrated.** [handle](/Users/ketansingrole/git/kchess/src/main/ipc.ts:5) validates method inputs through its callers but does not authenticate the sender/frame. Navigation and permission checks use URL prefixes. [setupMediaPermissions](/Users/ketansingrole/git/kchess/src/main/microphone.ts:61) grants every non-media permission. No explicit production Content Security Policy was found in the application configuration.

**Change:** centralize exact parsed origin checks; require the owned main window and intended top-level frame for privileged IPC. Deny unneeded permissions and validate requesting origins. Add a production CSP that deliberately supports local styles/assets, the blob worker and WASM needed by Vosk; keep development HMR policy separate. Test it against the packaged protocol, not just localhost. Keep shell URLs allowlisted.

The [Electron security guide](https://www.electronjs.org/docs/latest/tutorial/security) recommends sender validation, CSP and restrictive navigation/permissions. The existing sandbox and input validation are strong foundations, but they address different parts of the boundary.

### 9. P2 — Ordinary keyboard users cannot make board moves

**Code finding.** [ChessBoard](/Users/ketansingrole/git/kchess/app/components/ChessBoard.vue:162) provides a graphical board and promotion buttons but no keyboard move entry or accessible square interaction. Arrow keys navigate history, and voice is available in several training flows, but neither provides a general keyboard-only move path. The custom promotion dialog focuses its first button without a focus trap or restoration behavior. Global reduced-motion CSS does not feed the animation setting passed to Chessground.

**Change:** start with SAN/UCI move entry and a polite move/status announcement. Add a roving-focus square grid or equivalent accessible board control after validating the interaction with users. Use the existing dialog primitive for promotion, or implement trapping, inert background and focus restoration. Feed the system reduced-motion preference into Chessground's duration. Preserve existing annotations and legal-move feedback.

Verify keyboard-only play, promotion, reverse orientation, voice-disabled use, modal shortcuts and reduced motion. Automated accessibility checks are useful, but a screen-reader walkthrough is also necessary.

## Framework and library decisions

Versions below came from the installed dependency tree and live npm metadata during this review. “Latest” is a registry snapshot, not a reason by itself to upgrade.

| Area                                              | Installed / researched                               | Decision                                                                                                                                                                  |
| ------------------------------------------------- | ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| TypeScript                                        | 5.9.3; 6.0.3 trial passed; latest 7.0.2              | Adopt 6 in a focused compatibility change; retain Vue and ESLint checks. Trial native 7 separately for plain Electron/shared TS only if measured build time justifies it. |
| Vue                                               | 3.5.43; latest stable 3.5.43; 3.6 is RC              | Keep 3.5.43. No demonstrated need for an RC or a frontend rewrite.                                                                                                        |
| Nuxt                                              | 4.5.2                                                | Keep. Already using client rendering and route splitting. Profile before attempting removal.                                                                              |
| Nuxt UI                                           | 4.11.2 → 4.11.3                                      | Adopt the patch after UI regression checks. Prefer its existing accessible primitives and theme tokens.                                                                   |
| Vite                                              | Root 7.3.6; Nuxt renderer actually uses nested 8.3.1 | Preserve the split for now. Root Vite 8 conflicts with electron-vite 5's declared peer range.                                                                             |
| electron-vite                                     | 5.0.0; peer range Vite 5/6/7                         | Keep pending explicit Vite 8 support.                                                                                                                                     |
| Vitest                                            | 4.1.11; latest 5.0.3                                 | Keep the working version until the test/build compatibility change is deliberate.                                                                                         |
| Electron                                          | 44.5.0 → 44.5.1                                      | Adopt patch with startup, voice, engine and packaged checks.                                                                                                              |
| Pinia / VueUse                                    | 4.0.3 / 15.0.0                                       | Keep. Improve ownership and state shape rather than replacing them.                                                                                                       |
| Chessground                                       | 10.4.0 → 10.4.1                                      | Keep and take compatible patch; regression-test promotion, premoves, dragging and cleanup.                                                                                |
| chessops                                          | 0.15.1                                               | Keep. Extend PGN preservation and semantic validation using the existing library.                                                                                         |
| Lichess types                                     | 2.0.174 → 2.0.175                                    | Take patch with API fixtures and typechecking. Runtime validation remains necessary.                                                                                      |
| Unovis                                            | 1.7.0 → 1.7.1 for both packages                      | Update together if desired. No evidence that replacing the chart library is a priority.                                                                                   |
| Valibot / openapi-fetch / lru-cache / node:sqlite | Already used                                         | Keep. They cover the current needs without an ORM or another schema/cache library.                                                                                        |

**TypeScript nuance:** TypeScript 7 is stable, but Microsoft documents missing compiler-API support and the resulting Vue/Volar limitation. It offers side-by-side TypeScript 6 compatibility packages. See the [TypeScript 7 announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/). The local Vue checker still patches the JavaScript `tsc` implementation. Its broad peer range does not prove native-compiler compatibility.

The TypeScript 6 trial is encouraging, but an installed migration still needs `npm run check`, build and UI tests. If adopting the compatibility alias/native sidecar, update `scripts/build.mjs` deliberately: it currently hardcodes compiler package entry paths. A newer TypeScript checker improves development feedback; it does not directly speed the running chess UI. Use [TypeScript 6 release notes](https://www.typescriptlang.org/docs/handbook/release-notes/typescript-6-0.html) to resolve configuration changes rather than broadly suppressing diagnostics.

**Nuxt/Vite nuance:** the fresh build printed Nuxt 4.5.2 / Vite 8.3.1 for the renderer, followed by Vite 7.3.6 for electron-vite. KChess already receives Vite 8 renderer benefits. Upgrading the root package would affect a different part of the toolchain. The installed and latest electron-vite package metadata both exclude Vite 8. Nuxt's [4.5 announcement](https://nuxt.com/blog/v4-5) documents the renderer/build changes. Consider its build profiling and shared watcher only after measuring startup/file-watching costs.

**Nuxt UI:** the [4.11.3 notes](https://ui.nuxt.com/docs/releases/v4.11.3) include search/sidebar dialog-label accessibility fixes relevant to the components KChess uses. Existing Nuxt UI releases provide offline icon bundling and optional virtualized controls. This app already bundles 43 local icons, about 10.4 KB uncompressed. Keep that property. History renders pages of 20 games, so virtualizing its visible list is not the first bottleneck; query/payload size matters more.

### Optional libraries and alternatives

| Candidate                                    | Recommendation and reason                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Effect                                       | Consider one isolated backend experiment only. Its resource scopes, cancellation, typed errors and schedules fit UCI/network ownership, but adopting it throughout the app would be a substantial new programming model. Fix the concrete failures with ordinary TypeScript first, or trial one UCI service and compare readability, cleanup and tests. Do not simultaneously rewrite stores or replace Valibot. [Effect docs](https://effect.website/docs/v3/getting-started). |
| XState                                       | Useful if OnlineSession's transition complexity keeps growing. A typed reducer/state union may be sufficient now. Choose one lifecycle approach in a pilot rather than introducing both Effect and XState across the project. [XState docs](https://stately.ai/docs).                                                                                                                                                                                                           |
| fast-check                                   | Good development-only addition for PGN round trips, legal-move histories, cancellation sequences and migration invariants. Add it where generated cases reveal failures beyond existing examples. [Property-based testing docs](https://fast-check.dev/docs/introduction/why-property-based/).                                                                                                                                                                                  |
| Additional query/cache library               | Defer. Main already has persistent caching and LRU deduplication. First make one service own requests and stale/fresh metadata.                                                                                                                                                                                                                                                                                                                                                 |
| Animation library such as Motion for Vue     | Defer. Chessground already animates pieces and CSS covers transitions. Add only for a specific interaction needing interruptible motion; prioritize stable layout and reduced motion. [Motion docs](https://motion.dev/docs/vue).                                                                                                                                                                                                                                               |
| React, Svelte or a different component suite | No demonstrated benefit. Would replace working routes, state, UI and tests without solving main-thread work or UCI hangs.                                                                                                                                                                                                                                                                                                                                                       |
| Plain Vue + Vite instead of Nuxt             | Possible later simplification because SSR is disabled. Requires replacing auto-imports, routing, head/UI modules and tooling. Only proceed after a prototype demonstrates material startup/build savings.                                                                                                                                                                                                                                                                       |
| Tauri/Rust migration                         | Defer. Tauri uses system WebViews and Rust integration, so the work affects SQLite, UCI children, OAuth, updates, permissions and voice across platforms. Package-size savings are plausible; compatibility and user-experience gains were not measured here. [Tauri architecture](https://v2.tauri.app/start/).                                                                                                                                                                |
| Bun/pnpm/another package manager             | Defer. The Node/npm environment is pinned and CI works. Package-manager speed does not resolve app responsiveness.                                                                                                                                                                                                                                                                                                                                                              |

## Engine capabilities and resource scheduling

The bundled engine is **Stockfish 19 lite**, not the full-strength native engine. The [Stockfish.js repository](https://github.com/nmrugg/stockfish.js) explicitly distinguishes lite from its substantially larger full build. Keep lite as an immediate offline fallback and label its role clearly. Offer verified native Stockfish as an optional strength/performance upgrade on Windows/Linux as well as macOS. The current managed downloader supports macOS only.

The [Stockfish 19 release](https://stockfishchess.org/blog/2026/stockfish-19/) adds universal binaries and stricter FEN/UCI validation. Validate semantic positions and move histories before sending commands, capture critical error details, and use a fallback if a downloaded/native engine fails startup. Benchmark fixed positions and time budgets before claiming a speed multiplier.

[bestMove](/Users/ketansingrole/git/kchess/src/main/engine.ts:106) starts and kills a process on every reply. Analysis and review each maintain another engine. A manual review launched from the analysis page can therefore compete with the live analysis engine; the background-only busy rule does not cap all engines' combined threads. Renderer epochs discard results after takeback/new game, but there is no renderer cancellation method for the previous computer search.

**Highest-impact architecture change:** a shared UCI controller plus a scheduler governing total threads, memory and priority. Share the protocol implementation; separate game/search contexts where necessary. It need not mean one physical process for every feature.

- Keep a warm opponent engine during a computer game, with explicit reset/configuration and idle release.
- Prioritize interactive board work; allow review to pause and retain progress. Set a total CPU/memory budget with an optional battery-saving mode.
- Make cancellation reach main immediately. Use generation IDs assigned before asynchronous engine lookup so newer requests cannot be overtaken by slower setup.
- Send root FEN plus the move sequence for analysis/review where history matters. An isolated FEN loses previous repetitions; cache context must reflect that too.
- Store engine identity/version, search settings and achieved quality with reviews. A cache keyed only by game moves cannot support deliberate stronger reanalysis well.
- Distinguish provisional quick-pass judgments from completed review and show actual achieved depth. Current complete/depth fields do not fully communicate quality.

For more human-like opponents, experiment with Maia-3 behind the same engine interface after the controller is stable. The [official Maia-3 implementation](https://github.com/CSSLab/maia3) targets human move prediction; that is a different goal from Stockfish's best-move search. Check model distribution, supported deployment and rating calibration before bundling. Do not present random weak Stockfish moves as a validated human Elo model. A GPU-oriented engine is not a universal default for a small desktop app.

Engine suggestions, opening lookup, tablebases and a future AI coach should be unavailable for a live human Lichess game. Current background-review checks pause automatic work, but analysis and manually requested reviews have no equivalent main-process gate. Enforce that boundary in main as well as the UI. The [Lichess fair-play rules](https://lichess.org/page/fair-play) prohibit engine or software move assistance during relevant games.

## Usability and capability priorities

| Priority   | Change                                        | User benefit / implementation boundary                                                                                                                     |
| ---------- | --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| First      | Persist computer games and analysis documents | Recover work after closing/crashing. Include start position, headers, variations and last selection; schema-version the saved document.                    |
| First      | Visible connection and operation recovery     | Clear reconnect/retry/auth-required actions; retain useful cached data and user edits. Generic toasts should supplement the affected panel.                |
| First      | Reliable PGN import/export and keyboard moves | Makes the app useful for studies, sharing and keyboard users.                                                                                              |
| Next       | Review → practice your mistakes               | Convert verified turning points into local exercises, with progressive hints and a spaced revisit queue. Existing review/puzzle components can support it. |
| Next       | Cross-platform native engine manager          | Stronger analysis without increasing every install's payload. Show engine source, version and actual readiness.                                            |
| Next       | Study/comment support and saved collections   | Makes the variation board a durable learning tool rather than a temporary scratchpad.                                                                      |
| Later      | Opening explorer and endgame tablebase lookup | Add bounded caches, explicit network states and offline behavior; disable live-game assistance.                                                            |
| Experiment | Human-like opponents and grounded coach       | Evaluate learning value, responsiveness and deployment size before making them defaults.                                                                   |

The largest generated JS chunk was 5,785,934 bytes and contained Vosk. The source dynamically imports Vosk only after voice is enabled, so the build's generic large-chunk warning is not proof that initial startup loads it. Chart chunks were roughly 208–231 KB and entry JS about 413 KB. Keep the voice boundary lazy; measure voice activation and route startup separately. There is no evidence yet that replacing Nuxt UI or Unovis would improve the user's experience.

[loadData](/Users/ketansingrole/git/kchess/src/main/store.ts:453) still reads and sends all stored game-list rows. The per-account cap is 5,000, and followed players can multiply the total. Renderer pagination reduces DOM size but not database reads, IPC cloning, array proxying or filtering. Introduce a paged game-query API and SQL aggregates when profiling a large library demonstrates the need. Keep the existing on-demand PGN fetch. Use Vue `shallowRef` for deliberately immutable large snapshots only with correct replacement semantics; do not blanket-convert the mutable variation tree. These techniques align with [Vue's performance guidance](https://vuejs.org/guide/best-practices/performance.html).

Voice should keep its offline and explicit-enable behavior. First evaluate square/move accuracy, correction frequency, latency and false confirmations across accents and noise. A Sherpa-ONNX trial is reasonable for multilingual/local recognition, but it is not a proven upgrade for this app. Its [official documentation](https://k2-fsa.github.io/sherpa/onnx/index.html) describes local inference and platform support. Compare the same consented test corpus and include package/model size and cleanup behavior in the decision.

## Dependency audit interpretation

The full audit's 21 entries are affected packages along dependency chains, not 21 independent vulnerabilities. The concrete underlying advisories include [braces](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm), [http-cache-semantics](https://github.com/advisories/GHSA-ch52-4w7c-c8xp), [node-forge](https://github.com/advisories/GHSA-86w9-cpqp-85rv) and [uuid](https://github.com/advisories/GHSA-w5hq-g745-h8pq).

Most reported chains are development/build tooling. `npm audit --omit=dev` reported none, but this is not a complete shipped-code test: renderer libraries, including `vosk-browser` and its uuid dependency, are declared as development dependencies and bundled into the renderer. Review the actual call paths and emitted code before concluding reachability. The uuid finding concerns particular buffered APIs; this review did not establish an exploitable call in KChess.

The audit suggested older Nuxt/electron-builder versions as automatic fixes, and npm's electron-builder `latest` tag was lower than the installed version. Do not run `npm audit fix --force`, downgrade Nuxt or trust `latest` blindly. Inspect patched underlying packages and maintainer compatibility, update the lockfile in a dedicated change, and run checks/build/package tests. Track remaining findings with a documented reachability assessment and bounded follow-up, rather than dismissing all dev dependencies or blocking indefinitely on a count.

## Making development and AI-assisted changes more reliable

The current tooling is a good base. Improvements should make correct changes smaller and failures easier to reproduce:

1. Split the 1,041-line app store and 1,304-line settings page by stable ownership: app shell/navigation, accounts/profile, settings persistence, engine configuration and focused settings sections. Preserve the computer/online/history separation already started. Keep subscription/disposal ownership explicit; the review store currently discards subscription cleanup functions.
2. Introduce typed operation states and errors across IPC, including cancelled/retryable/auth-required. Replace the single global `busy` boolean with operation-owned state; overlapping actions currently let the first completion clear it while another is active.
3. Extend runtime validation to network/cache boundaries. OpenAPI interfaces disappear at runtime; several streamed payloads are direct casts after `JSON.parse`. Validate critical game-state/puzzle fields without copying every remote schema. Bound malformed NDJSON lines and PGN input sizes.
4. Add focused architectural notes showing where state/resources belong and which regression command covers each flow. Expand AGENTS.md with those invariants and fixture guidance, while retaining the existing release and dev-session rules.
5. Use current official Nuxt/Nuxt UI documentation with agents. Nuxt UI provides documented AI tooling and MCP integration in its [v4 announcement](https://ui.nuxt.com/docs/releases/v4.0.0). Make documentation accessible without changing application dependencies or exposing account tokens.
6. Add deterministic protocol fixtures and fault injection. Existing happy-path Electron tests are valuable; failures during stop/reconnect/import need dedicated coverage. Reuse real stores and the branded isolated launcher.
7. Consider stricter indexed-access/template checks incrementally, scoped to the shared contracts and services first. Existing strict Electron TypeScript and promise linting already provide value. More type assertions are not a substitute for validating external data.
8. Measure startup-to-interactive, board input-to-paint, engine startup/search/stop, main event-loop delay, review throughput and memory after repeated navigation. Use production builds and controlled profiles. Save bounded timing summaries in diagnostics, excluding tokens and identifying account data.

For an AI coach, let chessops validate all moves and Stockfish supply verified variations. Give a language model structured positions, scores, candidate lines and identified motifs to explain; do not ask it to invent chess facts or mutate arbitrary desktop state. Start with deterministic explanations for common mistakes, then compare a model-assisted version on a fixed expert-reviewed set. Keep the provider behind a main-process adapter with cancellation and bounded context. No agent framework is necessary for the first prototype.

## Suggested implementation order and acceptance criteria

| Sequence | Scope                                               | Acceptance criteria                                                                                                                                                                         |
| -------- | --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1        | Puzzle replacement + PGN preservation               | Failed replacements retain prior data; semantic PGN round trips preserve headers, result, annotations and variations.                                                                       |
| 2        | UCI lifecycle controller + analysis cache semantics | Hung engines fail visibly; stop/cancel has a bounded outcome; stale output cannot update a new position; interrupted results can resume; switching engines invalidates quality assumptions. |
| 3        | Online operation states + request policy            | Stream failures are visible; reconnect resumes authoritative state; polls never overlap or write to an old session; moves receive foreground priority.                                      |
| 4        | Bulk-work isolation + large-library measurements    | Import does not block main for the duration of validation/insertion; transaction and cancellation behavior stay correct; status reads avoid full-table scans.                               |
| 5        | Maintenance patches + TypeScript 6                  | Installed graph passes format/lint/all typechecks/tests/build and platform package checks. Root Vite stays compatible with electron-vite.                                                   |
| 6        | Keyboard/modal accessibility + session persistence  | Keyboard-only game/promotion works; reduced motion affects pieces; a restart recovers a saved computer game and study.                                                                      |
| 7        | Stronger engines + training capabilities            | Verified downloads on supported platforms, fallback on failure, measured resource budgets and useful mistake-practice flows.                                                                |
| 8        | Optional Effect/XState/voice/coach experiments      | A small prototype demonstrates a specific improvement with measured costs before broader adoption.                                                                                          |

Take these as separate reviewable changes. The largest returns are preserving users' work, bounding asynchronous failures and keeping background work off the UI-critical path. The technology upgrades support those changes; they do not replace them.
