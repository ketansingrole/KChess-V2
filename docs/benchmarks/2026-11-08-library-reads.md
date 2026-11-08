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
