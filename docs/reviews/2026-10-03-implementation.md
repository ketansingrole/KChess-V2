# Review implementation and adoption decisions

3 October 2026. Implements the adopted changes from [the project review](2026-10-03-project-review.md). This is a local working-tree change against `1ff8fad`; the package remains `2026.10.3`. No commit, tag, publication or release was created.

## Adopted stack

| Area                        | Result                                                                                        |
| --------------------------- | --------------------------------------------------------------------------------------------- |
| TypeScript                  | Installed 6.0.3; renderer, Electron and Vue test-project checking retained                    |
| Nuxt UI                     | 4.11.3                                                                                        |
| Electron                    | 44.5.1                                                                                        |
| Chessground / Lichess types | 10.4.1 / 2.0.175                                                                              |
| Vue template checker        | vue-tsc 3.3.12                                                                                |
| Charts                      | Both Unovis packages 1.7.1                                                                    |
| Property testing            | fast-check 4.10.2, development only                                                           |
| Frameworks retained         | Vue 3.5.43, Nuxt 4.5.2, Pinia, chessops, Electron and the existing typed preload architecture |
| Build compatibility         | Root Vite 7 retained for electron-vite 5; Nuxt's renderer uses its nested Vite 8              |

Effect, XState and a different frontend/runtime are not added. The concrete lifecycle problems now have small owned controllers, cancellation, validation and regression coverage. TypeScript 7/native compiler adoption remains a separate compatibility/performance experiment; the supported installed migration here is TypeScript 6.

## Reliability and performance changes

| Finding                              | Implemented behavior                                                                                                                                                                              | Evidence                                                           |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| Destructive empty puzzle replacement | Replacement fails inside its transaction when no valid rows survive; duplicate counts reflect actual inserts; cancellation is checked before commit                                               | Puzzle smoke cases and production-worker fixture                   |
| Main-thread puzzle imports           | Dedicated worker owns sampling, validation, queries, import and vacuum in `puzzles.db`; main uses an asynchronous service                                                                         | Desktop/packaged tests and 100,000-row worker probe                |
| Expensive puzzle status reads        | Size metadata is computed during import; status/installation checks read one metadata row                                                                                                         | Smoke cases and worker probe                                       |
| PGN information loss                 | Root headers/results and move comments, starting comments and NAGs survive import/export; illegal recognized moves and multiple documents are rejected; size, depth and node limits bound imports | Semantic examples and 40 generated legal-game round trips          |
| Unbounded UCI waits                  | Shared controller bounds handshake/readiness/search/stop, propagates pipe/exit failures and escalates termination; deliberately infinite searches retain bounded initialization and stop          | Fault-injection UCI tests and real Stockfish flows                 |
| Competing engines                    | One priority scheduler owns active searches; computer replies precede analysis and reviews; up to four threads, capped at two on battery; idle engines close after two minutes                    | Scheduler tests and desktop engine/review flows                    |
| Computer engine cold starts          | Computer process remains warm between moves, with generation cancellation and serialized searches                                                                                                 | Engine-level and computer lifecycle tests                          |
| Wrong analysis cache assumptions     | Completed/interrupted/failed results differ; cache keys include root position plus move history; client IDs reject stale updates; configured-engine changes clear cached quality assumptions      | Analysis/store tests and desktop analysis                          |
| Stale review evaluations             | Local review records include the engine file identity; changed engines force new scores; cancellation preserves partial work                                                                      | Review queue and real engine tests                                 |
| Invisible online failures            | Typed connection states retain the playing account/game/session; disconnected clocks pause and input stops; reconnect waits for authoritative state                                               | Store/stream fixtures                                              |
| Ambiguous move acknowledgments       | No automatic mutation replay; failed confirmation freezes input and asks the user to reconnect to the server position                                                                             | Recovery regression                                                |
| Overlapping presence polls           | One request in flight, completion-based polling, stale session rejection and short main-process deduplication cache                                                                               | Deferred-response regression                                       |
| Request overload/stalls              | Shared request priority and 429 cooldown; bounded ordinary requests, stream records and heartbeat waits                                                                                           | Stream boundary tests; no automatic mutation retries               |
| Renderer privileges                  | Exact app origins, owned top-level IPC sender, restrictive permissions, production CSP                                                                                                            | Origin tests and packaged-protocol checks                          |
| Offline voice under CSP              | Build extracts Vosk's pinned embedded worker into a local asset. Only that worker allows JS evaluation; renderer documents use script hashes and prohibit arbitrary inline scripts/evaluation     | Offline voice and CSP checks in development-build and packaged app |
| Offline icons/audio                  | Conditional/data-array icons are included in the local bundle; local audio data URLs are permitted                                                                                                | Desktop console checks across core routes                          |
| Overlapping busy flags               | Operation count remains busy until every pending operation completes                                                                                                                              | Concurrent-operation regression                                    |

The first puzzle-worker launch copies the old sample into its own database after the legacy schema is migrated. A worker migration marker prevents deleted samples from being resurrected on later launches. The legacy copy remains as a fallback, so existing users can temporarily retain both copies on disk. Subsequent puzzle writes belong only to the worker; a `DatabaseSync` instance never crosses threads.

## User-facing capabilities

- Computer games and analysis documents restore from bounded, versioned local storage. Analysis restoration includes selection, orientation, variations and annotations. Saving failures are visible; PGN export remains the portable backup.
- The analysis page edits White/Black/Event/Result and comments on the selected position, and stores up to 50 named local studies within a bounded library.
- Completed reviews can add verified White/Black mistakes to Practice → Your mistakes. Exercises retain legal engine lines, revisit successes at increasing intervals and return failures after ten minutes. Local evaluations below depth 10 and incomplete reviews do not create exercises.
- Review explanations use actual scores and legally replayed preferred lines, identify the source/depth and mark incomplete work provisional. No language model invents chess facts.
- Boards accept keyboard SAN/UCI moves, announce submitted moves and turns, trap promotion focus and restore it, and respect reduced motion in Chessground. A full screen-reader square-navigation interaction still requires user evaluation.
- The native engine manager selects conservative official builds for supported macOS/Windows/Linux architectures, verifies SHA-256 and probes UCI before replacement. Downloads remain optional. A failing user-selected executable reports an actionable error; it is not silently presented as a different engine.
- Opening statistics and seven-piece tablebases are explicit analysis-page lookups. Moves are validated and SAN is derived locally. Lookups have a 15-second deadline, bounded response size, pending-request deduplication and at most 256 persisted cache entries. Failed refreshes visibly reuse validated saved results. Opening lookup uses a connected account's token only in main and only for the official explorer; tablebases receive no token.
- Tablebase results distinguish fifty-move categories and DTZ from mate distance. Move outcomes are displayed from the player's perspective, reversing the child position's side-to-move result.
- Main gates engine searches, reviews and position lookups during a tracked live human game. Cached analysis/review output is hidden in the renderer. Losing/cancelling its stream does not clear that gate; the server must confirm the game ended or that no game remains.

The official [opening explorer specification](https://raw.githubusercontent.com/lichess-org/api/master/doc/specs/tags/openingexplorer/lichess.yaml) now specifies OAuth; an unauthenticated local probe returned HTTP 401. The [tablebase specification](https://raw.githubusercontent.com/lichess-org/api/master/doc/specs/tags/tablebase/standard.yaml) identifies the public endpoint. Lookup tests use controlled fixtures and offline saved results rather than depending on a changing public database.

## Development and measurement

Engine configuration, serialized settings persistence and account-profile refresh now have separate renderer owners under `app/stores/kchess/`, alongside the existing game/history helpers. The engine/review settings section is a focused component. Profile generations prevent an older refresh for the same account from replacing newer data. Subscription/disposal ownership and architectural invariants are documented in AGENTS.md.

Diagnostics retain bounded aggregates for engine startup/search/stop, renderer readiness, voice activation, next-frame delay after board submission, and main event-loop delay. Timing IPC accepts three fixed renderer names and finite bounded values; positions, account names and credentials are not timing dimensions. `board.frame` measures the next frame callback, not a physical display's paint completion. `app.ready` measures renderer navigation-to-store readiness, not OS process launch-to-interactive. These provide a baseline, not a claim of universally smooth performance.

`npm run test:puzzle-worker` requires a fresh build and uses an isolated on-disk database plus loopback HTTP. A 100,000-row production-worker run took 423 ms with a 12 ms maximum host heartbeat gap. A second run while desktop packaging verification ran took 711 ms with a 19 ms gap. Both preserved the old sample on invalid and cancelled replacements. These are single-machine synthetic probes, not comparative production benchmarks; the review's prior 539 ms synchronous probe used in-memory SQLite and is not an equivalent throughput baseline.

`npm run benchmark:library` measures synthetic in-memory SQLite row reads and structured cloning, with a 100-row SQL query for comparison. It excludes renderer work, PGN parsing, IPC transport and JSON serialization from query timing. Results on this machine:

| Rows   | Full SQL read | Full structured clone | Serialized snapshot | 100-row SQL read |
| ------ | ------------- | --------------------- | ------------------- | ---------------- |
| 1,000  | 1.15 ms       | 0.97 ms               | 0.30 MB             | 0.10 ms          |
| 5,000  | 5.37 ms       | 4.34 ms               | 1.52 MB             | 0.11 ms          |
| 50,000 | 54.38 ms      | 52.69 ms              | 15.20 MB            | 0.12 ms          |

The approximately 107 ms read-plus-clone cost at 50,000 rows supports prioritizing SQL paging and aggregates. It does not measure end-to-end dashboard latency or establish a production limit. Full-library snapshot loading remains in this implementation.

Use [Nuxt documentation](https://nuxt.com/docs), [Nuxt UI documentation](https://ui.nuxt.com/docs) and [Vue performance guidance](https://vuejs.org/guide/best-practices/performance.html) as the source for agent-assisted changes. Adding application libraries or an agent framework is not required to make those references accessible.

## Validation

Final local verification results:

| Command                      | Result                                                                       |
| ---------------------------- | ---------------------------------------------------------------------------- |
| `npm run check`              | Passed: formatting, lint, typechecks, smoke tests and 229 unit tests         |
| `npm run build`              | Passed; renderer 6.9 MiB versus reviewed 8.2 MiB                             |
| `npm run test:puzzle-worker` | Passed: 100,000 rows, invalid replacement rollback, import cancellation      |
| `npm run test:e2e`           | 14 passed against the final build                                            |
| `npm run pack`               | Passed: macOS arm64 package; 18.7 MiB application payload excluding Electron |
| `npm run test:packaged`      | 7 passed against the final package                                           |

Windows/Linux behavior is covered by the existing CI matrix and platform asset-selection fixtures, but was not executed on those operating systems in this session. No signed installer/update, live human game, real-speaker recognition corpus or screen-reader walkthrough was performed. The macOS package is unsigned, as configured in this repository. No remote CI status is claimed.

## Remaining upstream advisories

The final full dependency audit still reports 21 affected package entries: 19 high, 2 moderate, zero critical. Counts include parent chains, not 21 distinct vulnerabilities. `npm audit --omit=dev` is insufficient to establish shipped safety because this repository bundles some renderer dependencies declared as development dependencies.

| Underlying advisory                                                                                  | Local assessment and next action                                                                                                                                              |
| ---------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [braces: recursive-pattern stack exhaustion](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm)      | Present through build/glob tooling; current registry release remains affected. Keep build patterns under project control and adopt a maintained patch when published.         |
| [http-cache-semantics: max-stale cache isolation](https://github.com/advisories/GHSA-ch52-4w7c-c8xp) | Dependency of build/download tooling; KChess's application request layer does not use this HTTP cache. Current published release remains affected.                            |
| [node-forge: RSA signature verification](https://github.com/advisories/GHSA-86w9-cpqp-85rv)          | Signing/build-tool chain; current published release remains affected. This does not establish that every downstream build usage is unreachable.                               |
| [uuid: buffered v3/v5/v6 bounds](https://github.com/advisories/GHSA-w5hq-g745-h8pq)                  | No longer shipped (2026-10-21): the bundled UUID code came with `vosk-browser`'s worker, which was removed when speech recognition moved to the native `kchess-voice` helper. |

No forced audit fix or framework downgrade was applied. This assessment explains the observed call paths and remaining exposure; it is not proof of exploit absence. Adopt upstream fixes when available and repeat package/voice regression checks.

Revisit gate: re-run the full audit monthly (CI `audit` job stays non-blocking by design) and update this table when an advisory gains a patched release; do not let entries rot without a dated re-check.

## Experiments and conditional work kept separate

The review recommended experiments and measurement gates, not automatic replacement of every named technology. These remain explicitly separate from the adopted implementation:

| Candidate                            | Decision / acceptance gate                                                                                                                                                                                           |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| TypeScript 7/native compiler         | Measure Electron/shared compilation separately and prove Vue/ESLint/tool compatibility before changing the installed compiler.                                                                                       |
| Effect or XState                     | Consider one bounded service/state-machine pilot only when it reduces complexity beyond the owned controllers now in place; no repository-wide rewrite.                                                              |
| Maia-3 or other human-like opponents | Validate deployable model/runtime, distribution rights, rating calibration, memory/latency and expert-rated playing style before bundling. No unsupported Elo claims.                                                |
| Sherpa-ONNX voice                    | Compare the same consented speech corpus across accents/noise, move accuracy, false confirmation, latency and model/package size. Such a corpus is not available in this repository.                                 |
| Model-assisted coach                 | The deterministic grounded prototype is implemented. A provider/model version still needs an expert-reviewed evaluation set, provider/privacy preferences and a bounded main adapter; it is not silently enabled.    |
| Large game-library paging            | The synthetic probe below identifies snapshot cost; SQL paging/aggregates and coordinated dashboard, chart, history and profile changes remain follow-up work. Renderer pagination alone does not fix IPC/data cost. |
| Full accessible square navigation    | Keyboard move entry and promotion/reduced-motion fixes are implemented. Validate the richer square interaction with keyboard/screen-reader users before replacing the board control.                                 |
| Further shell/settings decomposition | Stable engine, persistence and profile owners are extracted; the shell and other settings sections can be split further without changing public store contracts.                                                     |

These are remaining evaluation/work items, not claimed completed capabilities or promises of an error-free application.
