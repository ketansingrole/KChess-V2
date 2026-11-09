# Library reads: what starting the app and opening a board page cost over a full profile

**Status: complete.** Linux, 4 cores, release builds (`kchess-core`'s fat LTO).

## Where the time went

The core's `library` answers every study, played game, mistake drill, unfinished session and
repertoire note at once. Reading it decodes every study and **replays every played game move by
move** (semantic validation), so over a full profile (the limit of 500 played games, here of 80
moves each, and 50 studies) one read takes 30 ms, 96% of it replaying the games.

The app read it on the main thread, blocking, before its window opened: once to check the profile
is readable, then once each for the played games and the studies. Over the board and the Analysis
board read it again, blocking, when they open (to resume their sessions), and Home, Play with
computer and Practice read it in the background.

## The fixes

- **The core keeps the snapshot** (`store::StoreCache::library`,
  `store::library_import::cached_library`), tagged with SQLite's count of rows its connection has
  changed (`total_changes`). Any write by any storage method, or none, changes the count, so the
  snapshot is answered only while nothing has been written since it was read. The core holds the
  profile's only connection to `kchess.db`; a connection opened again counts from zero, so opening
  one forgets the snapshot. Each read still leaves out tournaments that ended by its own time.
- **The app reads it once to start** (`startup::check` keeps what it read, and `played::init` and
  `studies::init` start from it).

## Results

`cargo test --release -p kchess-core --test store_speed -- --ignored --nocapture`; medians of 15.

| Read over a full profile (451 KiB of JSON) | Before | After | Change |
| ------------------------------------------ | -----: | ----: | -----: |
| `library`, read again, written as JSON     | 30.2 ms | 5.3 ms | **5.7×** |
| `library`, first read (or after a write)   | 30.2 ms | 30.2 ms | — |
| Reads of it before the window opens        | 3 | 1 | 3× |

Before the window opens, the app now spends one 30 ms read where it spent three (about 90 ms).
Opening Over the board or the Analysis board blocks for the 5 ms of a kept read instead of 30 ms
(unless a game was saved since).

## Follow-up: every page opened, measured and profiled

`e2e::perf` starts the app over the same full profile and opens every page (first, then again),
headless, so it times the main thread's work, not the GPU's. Each slow opening was profiled
(`perf record -g`, frame pointers; `KCHESS_PERF_PAGES` opens one page) before it was changed.

The first change made the Analysis board's first opening slower (49 → 59 ms): Over the board saves
its session as it opens, so the Analysis board after it missed the kept snapshot, and a miss also
copied the snapshot to keep it. The profiles then showed where the openings went:

| Opening | Where the time went | Fix |
| ------- | ------------------- | --- |
| Over the board, first | 57% in the game-history rules, mostly copying every played game: they read the list to find the game's own entry | The rules get only that entry (`game/history.rs`, `entries_of`); they look a game up by its identity's id alone |
| Analysis board, first | Reading the whole library to resume one session | `session(kind)` and `mistakes()`, new core reads; Over the board reads its three sessions with it |
| Studies | 83% of drawing the page in `summarizeStudy`, twice per study per frame | Summaries kept by PGN while the study is listed |
| Game history | Half of drawing the page in copying the 500 games, each frame | The list borrows the played games |

`cargo test --release -p kchess-gpui e2e::perf -- --ignored --nocapture` (no LTO, 16 codegen
units, both sides alike); `main` at `30627a8` against this change, medians of 3 alternating runs.

| Opening | main | After | Change |
| ------- | ---: | ----: | -----: |
| Over the board, first | 97.3 ms | 10.5 ms | **9.3×** |
| Analysis board, first | 46.6 ms | 12.4 ms | **3.8×** |
| Studies, again | 34.8 ms | 24.0 ms | 1.45× |
| Game history, first | 21.5 ms | 17.0 ms | 1.26× |
| Game history, again | 19.8 ms | 15.2 ms | 1.30× |
| Studies, first | 37.3 ms | 35.6 ms | — |

Studies' first opening still summarizes every study once. The other pages, and starting the app
(58–62 ms to the window, from 139 ms before the first change), are unchanged within the runs'
spread; Openings' first opening moved with the pages opened before it (13–23 ms either way).

## Looked at and left

**Pages of synced games, newest to oldest.** Game history pages with `LIMIT … OFFSET …`, which
walks every skipped row, so older pages are slower: over 50,000 games, the newest page takes
0.4 ms and the 1,000th 1.8 ms (every account) or 3.2 ms (one account). Keyset paging would make
them flat, but at 3 ms there is nothing a player could notice.

## Reproduce

```bash
cargo test --release -p kchess-core --test store_speed -- --ignored --nocapture
cargo test --release -p kchess-gpui e2e::perf -- --ignored --nocapture
```
