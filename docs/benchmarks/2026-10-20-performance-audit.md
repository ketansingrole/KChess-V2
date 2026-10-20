# Performance audit and measured optimizations

**Date:** 2026-10-20  
**Baseline:** `main` at `c343f5a`  
**Branch:** `perf/audit`, reviewed, corrected and re-measured before merging (see
[Review before merging](#review-before-merging)).

This audit reviewed the Rust domain, the Rust core services, the TypeScript rules bindings, the
renderer stores/components and the Electron/main hot paths. It implemented the changes that could
be made safely and measured them against the unmodified `main` build. Candidate changes that need
a migration, a protocol change or more validation are listed separately in
[Findings not implemented](#findings-not-implemented).

## Environment

| Item       | Value                                                                           |
| ---------- | ------------------------------------------------------------------------------- |
| Machine    | Apple Silicon (arm64), macOS, 10 logical CPUs                                   |
| Node       | 24.21.0                                                                         |
| Rust       | 1.94.0, release profile (`lto = "fat"`, `codegen-units = 1`)                    |
| Benchmarks | `perf/*.mjs`, direct native/WASM rules calls and real core API calls            |
| Fixtures   | Deterministic local SQLite, PGN and loopback HTTP fixtures; no external network |
| Timing     | Warm calls, p50 unless noted; paired before/after runs on the same machine      |

`perf/bench-*.mjs` accept `--root=/path/to/checkout`, so the same script measures a `main`
checkout and the changed one. Give each checkout its own Cargo target directory: with a shared
one, `pnpm run build:native` in one checkout can copy the other checkout's library and silently
benchmark the wrong code. Results go to `perf/results/` (ignored by Git) or `PERF_OUT`.

## Executive summary

The largest measured wins are the ones on per-row and per-request paths:

| Area                                                 |                Before |                After |                                        Change |
| ---------------------------------------------------- | --------------------: | -------------------: | --------------------------------------------: |
| Insights, 50k games, 20k distinct opening families   |              189.8 ms |              24.8 ms |                                    **−86.9%** |
| Eight concurrent profile reads, same user            | 405.9 ms / 8 requests | 102.7 ms / 1 request |                  **−74.7% / −87.5% requests** |
| Filtered history page, 50k games (win + rated)       |               2.52 ms |              0.35 ms |                                    **−86.2%** |
| History page, 50k games                              |               1.04 ms |              0.34 ms |                                    **−66.9%** |
| Library overview, warm, 50k games                    |               12.0 ms |             0.028 ms |                                    **−99.8%** |
| Insights, 50k games, ordinary openings               |               28.2 ms |              23.3 ms |                                    **−17.3%** |
| `openingAt` (opening name per move)                  |               56.9 µs |              38.7 µs |                                    **−32.0%** |
| `explainReviewedMove` (120-ply review)               |               76.4 µs |              38.0 µs |                                    **−50.3%** |
| `materialDiff`                                       |                2.5 µs |               0.7 µs |                                    **−72.0%** |
| `openingTree` (openings page)                        |               5.10 ms |              3.10 ms |                                    **−39.2%** |
| Position + SAN history per move (40 plies)           |              0.934 ms |             0.627 ms |                                    **−32.8%** |
| Review-mark path building, 100 plies                 |              0.323 ms |             0.002 ms |                                    **−99.4%** |
| One analysis-line row (path, number, glyph, comment) |                0.7 µs |               0.1 µs |                                    **−85.7%** |
| `formatGameDate`, 100 history rows                   |              1.512 ms |             0.035 ms |                                    **−97.7%** |
| `gameResult`, 100 history rows                       |              0.625 ms |             0.048 ms |                                    **−92.3%** |
| `formatEval`, 100 engine-line renders                |              0.110 ms |             0.001 ms |                                    **−99.1%** |
| Store JSON clone per save                            |               removed |                    — | removes a full stringify/parse of the session |
| Chessground arrow update                             |    full `set(config)` |      `setAutoShapes` |      avoids FEN/dests/premove reconfiguration |

These numbers are separate microbenchmarks, not one composite score. They describe this host and
these fixtures, not every user or platform.

## Implemented changes

### 1. Insights grouping is linear again

**Files:** `crates/kchess-core/src/store/insights.rs`

The TypeScript implementation grouped speeds, opening families and endings in a `Map`. The Rust
port used a `Vec::position` linear scan for each row, which is O(rows × distinct keys). With 20k
distinct opening families and 50k games, that was 189.8 ms; a hash index alongside the ordered
vector keeps first-seen order and ties while making grouping O(rows).

The query also called `strftime` twice per row, once for weekday and once for hour, each with a
`localtime` conversion. One combined `strftime('%w %H', …, 'localtime')` expression is parsed in
Rust, so SQLite converts the timestamp once.

| Workload (50k games, warm)     |   Before |   After | Change |
| ------------------------------ | -------: | ------: | -----: |
| Insights, 20k opening families | 189.8 ms | 24.8 ms | −86.9% |
| Insights, ordinary openings    |  28.2 ms | 23.3 ms | −17.3% |

The ordinary-opening row varies with run order (8–17% across paired runs); the large win is the
removal of the quadratic grouping. `crates/kchess-core/tests/store_insights.rs` retains the
timezone-boundary parity oracle and the opening-order test.

### 2. History totals and library overview are cached until games change

**Files:** `crates/kchess-core/src/store/mod.rs`, `crates/kchess-core/src/store/games.rs`

A filtered `gamePage` spent almost all of its 2.52 ms obtaining the total: the page itself is a
short indexed range. Totals are now cached by account/result/rated in a bounded 128-entry map.
The global `gameLibraryOverview` is cached as one value because its `GROUP BY lower(account)`
scans every game.

`StoreCache` is cleared by every `store.games.*` method except a fixed list of reads, by imports
and debug SQL, and when the database closes. Listing the reads rather than the writes means a
new mutation, or one that only touches `accounts` (a login connecting an account changes the
overview's versus cards), cannot leave a stale value behind.

| Workload (50k games, warm) |  Before |    After | Change |
| -------------------------- | ------: | -------: | -----: |
| History page               | 1.04 ms |  0.34 ms | −66.9% |
| Filtered history page      | 2.52 ms |  0.35 ms | −86.2% |
| Library overview (cold)    | 12.0 ms |  10.8 ms |   −10% |
| Library overview (warm)    | 12.0 ms | 0.028 ms | −99.8% |

The overview query itself was also rewritten from `GROUP BY lower(account)` to
`GROUP BY account COLLATE NOCASE`, which lets SQLite walk `idx_games_account_created` instead of
building a temporary B-tree. That cut the cold call from about 11.4 to 8.1 ms before the cache
was added.

Regressions added in `crates/kchess-core/tests/store_games.rs`:
`filtered_page_totals_are_cached_until_the_games_change`,
`library_overview_is_cached_until_the_games_change` and
`library_overview_is_recomputed_after_a_login_connects_an_account`. They call the `Database`
directly: the test `Store` runs the one-time import before every call, which clears the cache
and would let these tests pass with any invalidation.

### 3. Concurrent profile reads share one request

**File:** `crates/kchess-core/src/lichess/accounts.rs`

`cached_fetch` previously checked the five-minute memory cache and then started a fresh HTTP
request on a miss. Eight simultaneous requests for the same user produced eight wire requests and
completed in about four latency waves. A bounded pending map now holds a `tokio::sync::OnceCell`
per `(account epoch, cache key)`; the first caller runs the request, the rest await it, and the
cell is removed after completion. Failed or cancelled leaders leave the cell uninitialized so the
next caller can take over. Logout epochs keep old requests from being reused. Past 256 entries,
cells no caller still holds are dropped.

| Workload (100 ms fixture latency)               |               Before |               After |                  Change |
| ----------------------------------------------- | -------------------: | ------------------: | ----------------------: |
| Eight concurrent `profile("FixtureUser")` calls | 405.9 ms, 8 requests | 102.7 ms, 1 request | −74.7%, −87.5% requests |

`concurrent_profile_reads_share_one_request` in
`crates/kchess-core/tests/lichess_accounts.rs` checks the wire count.

### 4. Opening lookup without per-ply strings

**Files:** `crates/kchess-domain/src/training/openings.rs`

`openingAt` replayed every ply through `position::line`, which builds a UCI string, a SAN string
and a FEN record per move, and then built a second repetition key per FEN with
`split(' ').take(4).collect().join(" ")`. The lookup now walks forward once, builds only the
borrowed repetition key (the first four FEN fields) and keeps the last named position. This is
the same “nearest named position” answer without the intermediate vectors and strings.

`openingTree` rebuilt and sorted the whole tree on every call. It is now built once in a
`OnceLock` and shared. The remaining ~3.1 ms is JSON serialization of the cached tree.

| Workload                      |  Before |   After | Change |
| ----------------------------- | ------: | ------: | -----: |
| `openingAt`, 120 plies        | 56.9 µs | 38.7 µs | −32.0% |
| `openingAt`, 40 growing plies |  453 µs |  342 µs | −24.5% |
| `openingTree`                 | 5.10 ms | 3.10 ms | −39.2% |

### 5. Review and material helpers stop rebuilding whole games

**Files:** `crates/kchess-domain/src/training/coach.rs`,
`crates/kchess-domain/src/replay.rs`, `crates/kchess-domain/src/voice.rs`,
`crates/kchess-domain/src/review.rs`

- `explainReviewedMove` called `replay_positions` for the entire game to read one FEN. A new
  `replay::fen_after(fen, moves, count)` replays only up to the selected move.
- `materialBalance`/`materialDiff` lowercased each character into a `String`, compared uppercase
  strings, and scanned the placement 12 more times for the per-role counts. One byte pass fills
  two `[u8; 6]` count arrays; the score and surplus lists come from those counts.
- `analyseReview` counted inaccuracy/mistake/blunder with six full passes (three labels × two
  sides) and allocated a `Vec<f64>` for every accuracy window. Counts are now one pass and the
  standard deviation sums directly over the window. The end-of-game check is skipped when every
  position already has a stored score.

| Workload                         |  Before |   After |             Change |
| -------------------------------- | ------: | ------: | -----------------: |
| `explainReviewedMove`, 120 plies | 76.4 µs | 38.0 µs |             −50.3% |
| `materialDiff`                   |  2.5 µs |  0.7 µs |             −72.0% |
| `analyseReview`, 120 plies       | 85.1 µs | 84.9 µs | no material change |

`analyseReview` is dominated by decoding the stored review and building its per-move JSON; the
allocation reductions did not move the p50 on this fixture. They are retained because they remove
allocation pressure at larger review sizes, and the golden parity suite still passes.

### 6. FEN and PGN writing without temporary strings

**Files:** `crates/kchess-domain/src/rules.rs`, `crates/kchess-domain/src/pgn.rs`,
`crates/kchess-domain/src/lichess/watch/feed.rs`

`make_fen` used `format!` for the remaining checks and the half-move/full-move counters on a path
taken for every played move. It now writes into the FEN buffer, and the en-passant square is two
`push` calls rather than `Square::to_string()`. `make_pgn` no longer allocates a `String` per move
number or NAG. `position_key` (TV feed) borrows the first two FEN fields instead of `split` +
`collect` + `join`.

| Workload                                |  Before |   After |        Change |
| --------------------------------------- | ------: | ------: | ------------: |
| Replay 120 plies (one FEN per position) | 77.7 µs | 72.5 µs |         −6.7% |
| `treeToPgn`, 120 plies                  | 96.8 µs | 95.2 µs | ~1.7% (noise) |

### 7. Renderer rules wrappers keep small pure results

**Files:** `crates/kchess-wasm/js/analysisTree.ts`, `crates/kchess-wasm/js/clock.ts`,
`crates/kchess-wasm/js/gameStatus.ts`, `crates/kchess-wasm/js/variant.ts`,
`crates/kchess-wasm/js/chess.ts`

Every `rules()` call serializes arguments, crosses the WASM boundary and parses JSON. Several
pure helpers were called once per row, per engine update or per clock tick:

- `moveNumber`, `commentText`, `moveGlyph`, `formatEval` and `winningChances` now have small
  bounded memos. The same values repeat across analysis lines, engine updates and chart renders.
- `gameResult` sent the entire `LichessGame` (moves + PGN) across the boundary for every history
  row; it now sends only `winner` and `color`, the fields the rule reads.
- `replaySetup` keeps the last replay of each move list (a `WeakMap` by array) and reuses it only
  when the setup and the full move text still match. `setupSanHistory` uses the same memo, so
  the position and the SAN list for one move share a single replay. A new array (a restart, even
  at the same position) still gets a new `Position`, which watchers rely on to see a change.

| Workload                                   |   Before |    After | Change |
| ------------------------------------------ | -------: | -------: | -----: |
| Position + SAN history per move (40 plies) | 0.934 ms | 0.627 ms | −32.8% |
| Repeated position + history (warm)         |  63.4 µs |   0.8 µs | −98.7% |
| `formatEval` ×100                          | 0.110 ms | 0.001 ms | −99.1% |
| Engine lines (3) per update                |   3.0 µs |   0.2 µs | −93.3% |
| `gameResult` ×100 rows                     | 0.625 ms | 0.048 ms | −92.3% |

`replaySetup_per_move` with a freshly allocated array each time is unchanged (0.554 → 0.553 ms
for 40 plies). The win comes from sharing between the position and history of the same stable
list, which is what the game stores ask for.

### 8. Analysis-line and review-mark paths are incremental

**Files:** `apps/desktop/app/components/AnalysisLine.vue`,
`apps/desktop/app/stores/analysis.ts`

Each `AnalysisLine` row called `pathOf([...movesOf(parentPath), node.uci])` — two rules calls
and several arrays per row. The path is now built from the parent path and the node's UCI.
`reviewMarks` built `pathOf(mainline.slice(0, i + 1))` for every reviewed ply while rebuilding
the whole map; it now appends each move to a running path.

| Workload                                       |   Before |    After | Change |
| ---------------------------------------------- | -------: | -------: | -----: |
| Analysis row (path + number + glyph + comment) |   0.7 µs |   0.1 µs | −85.7% |
| Review paths, 100 plies                        | 0.323 ms | 0.002 ms | −99.4% |

The review is still rebuilt on every streamed update; see
[Findings not implemented](#findings-not-implemented).

### 9. History formatting reuses one `Intl.DateTimeFormat`

**File:** `apps/desktop/app/utils/games.ts`

`formatGameDate` constructed a new `Intl.DateTimeFormat` for every call, twice per history row.
A module-level formatter is reused.

| Workload              |   Before |    After | Change |
| --------------------- | -------: | -------: | -----: |
| `formatGameDate` ×100 | 1.512 ms | 0.035 ms | −97.7% |

The same pattern remains in `puzzleAttempts.ts`, `format.ts`, `ratingChart.ts` and
`AppUpdateSettings.vue`; they are low-frequency but can share this helper.

### 10. Renderer saves and watchers stop cloning/deep-walking plain snapshots

**Files:** `apps/desktop/app/utils/library.ts`,
`apps/desktop/app/stores/gameArchive.ts`,
`apps/desktop/app/stores/kchess/settingsPersistence.ts`,
`apps/desktop/app/stores/kchess.ts`,
`apps/desktop/app/components/ChessBoard.vue`

- `persistSession` ran `plain(source())` (a full JSON stringify/parse) and watched with
  `deep: true`. The session source is already a plain rules snapshot; it is only cloned when the
  value is actually reactive (`archive` identity), and the watcher now tracks the getter's reads
  instead of walking the returned plain object. `useGameArchive` had the same redundant clone
  and deep watch.
- `settingsDirty` stringified both the edited and stored settings on every read. Both
  serializations are now cached by `computed`; the theme/audio watcher watches only the fields
  it applies instead of the whole settings object. Changing engine path, notifications or review
  options no longer rewrites every theme CSS variable.
- `ChessBoard` watchers now split arrow changes from position/dests changes. Engine PV updates
  call `setAutoShapes` instead of rebuilding the full Chessground configuration; the player's
  own right-click drawings (`setShapes`) are left alone (`board-shapes.test.ts`).

The save-path clone was 2.5 µs for a 714-byte test session; the cost scales with PGN size. The
headless Chessground probe measured ~4.5 µs for `set(config)` versus ~0.1–0.5 µs for
`setShapes` before paint; the real win is avoiding a full board reconfiguration on every PV
update, which is not captured by that synchronous number.

## Findings not implemented

These were confirmed in the code/measurements but need a migration, a protocol change or more
validation than this audit should land without a dedicated review.

| Priority | Finding                                                                          | Evidence / expected impact                                                                                                     | Minimal fix                                                                                                      |
| -------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------- |
| High     | `gameRatingHistory` sorts/numbers every rated game for a handful of daily points | 30.3–32.6 ms for 50k games; a grouped `MAX(createdAt)` query was 14.3 ms, an expression index plus the window query 20.5 ms    | Add a partial expression index or group in SQL/Rust while preserving the `id DESC` tie-break                     |
| High     | Review streaming rebuilds the whole review per update                            | `analyseReview` 85 µs and path rebuild 0.32 ms per 100 plies; a 74-ply review can do this ~74 times                            | Coalesce `review:update` per frame, memoize by key + highest scored ply, update marks incrementally              |
| High     | Analysis root has `watch(root, …, { deep: true })`                               | Traverses every node on each edit/comment keystroke                                                                            | Explicit revision bumps from play/remove/promote/comment/glyph paths, or debounce comment writes                 |
| Medium   | Archived games and studies are revalidated on every `library()` snapshot         | Max fixture (20 studies × 64 chapters + 500 games, 1.5 MB) took 31.8 ms; typical 20×20 + 200 games took 9.0 ms                 | Persist a validated content hash/schema version and revalidate only rows without it, or lazy-load archived games |
| Medium   | `assert_analysis_request` replays every ply and builds every FEN/legal-move set  | Runs on analysis start/review open; 200-ply history does 200 FENs, 200 legal-move generations                                  | Add `replay::last_position` and compare only the last FEN + move count                                           |
| Medium   | Main-process IPC validation rebuilds whole documents with `rulesLossless`        | Every `saveSession`/`saveArchivedGame` copies the full document again before Rust validates it                                 | Fast-path compact document shapes; keep full Rust validation on save                                             |
| Medium   | Startup loads WASM and the complete library sequentially                         | `loadLibrary` waits for the 1.5 MB snapshot; Nuxt awaits the rules plugin first                                                | Start the library IPC and WASM fetch concurrently; gate only rule-dependent UI on rules                          |
| Medium   | Insights report is not cached                                                    | 23–28 ms at 50k games on every page open                                                                                       | Cache by normalized query and day, invalidate on game/review writes                                              |
| Medium   | `openingTree` re-serializes the cached tree on every call                        | 3.1 ms remains after the cache                                                                                                 | Cache the JSON string at the Rust boundary or return a prebuilt payload                                          |
| Low      | A filtered page's count scans the filter's index on a cold cache                 | Page rows are 0.12 ms; the count is 2.5 ms; the warm cached page is 0.35 ms                                                    | The cache covers repeated reads; an index including `rated` and `createdAt` could serve both cold filters        |
| Low      | `gamePage` count cache is conservative                                           | Any non-game write does not clear it (correct), but the invalidation list must be kept in step with new game/account mutations | Keep `invalidates_counts` beside the dispatch and add new mutations there                                        |
| Low      | Two `MaterialDiff` components per board compute the same diff                    | Two calls per position change; now ~1 µs each after the Rust change                                                            | Compute once in the board page and pass the diff down                                                            |
| Low      | Clock ticks rebuild player objects                                               | 2 clocks × 4–10 ticks/s; `ClockText.vue` exists but is unused                                                                  | Render a clock leaf and keep the player objects stable                                                           |
| Low      | Large immutable lists use deep refs                                              | `LocalGameHistory.selected`, `players.profile/games/history`, `watch.frame`, `tournaments.list`                                | `shallowRef` (and `triggerRef` where a frame is mutated)                                                         |
| Low      | `positionKey`-style split/join remains in a few places                           | TV alignment and rating-key building                                                                                           | Borrow the first fields as `position_key` now does                                                               |
| Low      | Other date formatters construct `Intl` per call                                  | Puzzle attempts, rating chart, update settings                                                                                 | Reuse one module-level formatter                                                                                 |

## Verification

- `pnpm run check` passed after the review fixes: formatting, lint, Rust format/clippy/tests,
  TypeScript type checks, 141 unit suites / 1149 tests, and structural performance budgets.
- `KCHESS_FUZZ_SCALE=20` native parity suites (rules, training, voice, trainer) pass.
- `pnpm run build` completed, and 11 production UI tests passed: analysis with Stockfish,
  variations, game review, paged history, Insights, keyboard moves, appearance settings, the
  local game and clock, Chess960 against Stockfish, and the settings flush on quit.
- Golden parity suites (`tests/core/native-rules.test.ts`, native training/review suites) pass
  after the Rust changes; the native addon and WASM were rebuilt by `pnpm run build:native`.
- New Rust regressions:
  - concurrent profile reads share one request;
  - filtered page totals and library overview invalidate on writes.

## Caveats

- Microbenchmarks use local deterministic fixtures. They do not model real Lichess latency,
  real PGNs of every shape, Windows/Linux behavior or long-session memory.
- Core timings include JSON bridging and the native module boundary, as the app sees them.
- The overview warm number is a cache hit; the first call after launch or a sync still costs
  about 11 ms. The insights and library improvements are latency wins, not memory wins.
- The Chessground probe ran headless and did not measure painted frames; treat its numbers as
  synchronous API cost only.
- The `replaySetup` memo is safe for the append/takeback move lists used by the stores. A caller
  that mutates a list in place at the same length and with the same last two moves would see a
  stale replay; no current caller does that.

## Review before merging

The branch was re-measured against a clean `main` build (paired runs, same machine) and every
headline row above reproduced within a few percent; for example insights with 20k families
201.7 → 26.8 ms, filtered history 2.65 → 0.40 ms, warm overview 10.8 → 0.03 ms, eight
profile reads 407.9 ms / 8 requests → 102.6 ms / 1 request. The first re-measurement did not:
the shared Cargo target had put the branch's library into `main`, so `main` appeared to dedupe
requests already. Rebuilding `main` from its own sources fixed the baseline.

The review changed these parts of the original branch before merging:

| Change in the branch                                     | Problem                                                                                                  | Resolution                                                                    |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `formatClock` memo by display bucket                     | 1.5 s and 15 s shared bucket 15: the last ten seconds could show `0:15`                                  | Removed; the call costs ~0.4 µs                                               |
| `ChessBoard` arrows through `setShapes`                  | The app's arrows are auto shapes; `setShapes` replaced the player's own drawings on every engine update  | `setAutoShapes`, with `board-shapes.test.ts`                                  |
| Cache invalidation by a list of writes                   | `saveTokenCiphertext` (a login) connects an account but kept the cached overview                         | Invalidate on every games method except known reads; login regression added   |
| Cache regressions through `Store::ok`                    | The test helper's per-call import cleared the cache, so the tests passed with any invalidation           | Tests call the `Database` directly and prove a cache hit                      |
| `replaySetup` keyed by array identity and last two moves | An in-place edit of an earlier move returned a stale replay, and a hit tracked only three reactive reads | Reused only when the full move text matches; regression in `variants.test.ts` |
| `materialCounts` with `u8` counters                      | An unvalidated placement with more than 255 of one piece overflows                                       | `u32`, with a regression in `native-voice.test.ts`                            |
| Pending-request cleanup `retain(get().is_none())`        | Kept exactly the abandoned cells it meant to drop                                                        | Drops cells no caller still holds                                             |
| `perf/results/*.json` and `/private/tmp` paths           | Machine-specific output committed; macOS-only temporary paths                                            | Output ignored by Git; scripts use `os.tmpdir()`                              |

## Reproduce

From a checkout, with a separate `main` checkout as the baseline:

```bash
# Rules (native binding, includes the JSON boundary)
node perf/bench-rules.mjs --root=. --label=after
node perf/bench-rules.mjs --root=../kchess-main --label=before

# Core API (seeds a disposable 50k-game profile)
node perf/bench-core.mjs --root=. --label=after --games=50000 --runs=8

# Twenty thousand distinct opening families
node perf/bench-core.mjs --root=. --label=after20k --games=50000 \
  --opening-labels=20000 --runs=4

# Renderer helpers and rules wrappers
node perf/bench-renderer.mjs --root=. --label=after

# Concurrent profile reads against a 100 ms loopback fixture
node perf/bench-network.mjs --root=. --label=after --runs=3

# Library snapshot at the document limits
node perf/bench-library.mjs --label=max --studies=20 --chapters=64 --games=500 --plies=100
```

The scripts write their JSON to `perf/results/` (or `PERF_OUT`); it is not committed.
