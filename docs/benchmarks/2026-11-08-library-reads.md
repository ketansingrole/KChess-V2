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
(57–74 ms to the window either way), are unchanged within the runs' spread; Openings' first
opening moved with the pages opened before it (13–23 ms either way).

### Studies, first opening

`summarizeStudy` built each study's whole tree (SAN, UCI and a FEN for every move) to report
counts and the main line's last position. `tree::summarize_pgn` reads them from the parsed game
with the same move checks, writing only that one FEN; `records::studies` tests it against the
tree's summary, and the rules corpus's 158 recorded answers still hold. Opening Studies alone
(`KCHESS_PERF_PAGES=Studies,Home`), 5 alternating runs, medians:

| Studies | Before | After |
| ------- | -----: | ----: |
| First opening | 34.0 ms | 30.1 ms |
| Again | 20.8 ms | 21.3 ms |

What is left is drawing the 50 cards (the limit), 21 ms with nothing to summarize: 37% of the
first opening is laying them out. Drawing only the cards in view would take most of it, but cards
off screen would then be out of reach of Tab and screen readers until scrolled to.

### Studies: only the cards in view

After the summaries, laying out the 50 cards was 37% of opening the page, and every wheel step
re-rendered all of them. The page now makes the rows of cards its viewport shows and as much again
above and below; spaces as tall as the rows left out keep the grid's height. Each row records its
measured height for the card width it was laid out at: a row measured at the first frame's width
and then scrolled out of view made the grid 18.5 px short until this was added
(`e2e::studies::only_the_studies_in_view_are_made` checks the grid is as tall as its rows). Cards
out of view are not reachable by Tab or a screen reader until scrolled to; that trade was asked for.

`e2e::perf` now also times six 300 px wheel steps on every page. `main` (`dc566eb`) against this
change, medians of 3 alternating runs of the whole tour:

| Studies | main | After | Change |
| ------- | ---: | ----: | -----: |
| First opening | 33.7 ms | 16.1 ms | **2.1×** |
| Again | 26.4 ms | 12.5 ms | **2.1×** |
| A wheel step | 21.2 ms | 9.3 ms | **2.3×** |

No other page's figures moved beyond the runs' spread or a fraction of a millisecond.

### Game history: outcomes and opening names

Profiled over 500 played games, a wheel step of Game history spent 20% working out every game's
outcome (each frame, for the list and its filters' counts) and its rows copied each listed game,
moves and all, four times a frame; its first opening spent 16% naming openings.

- The page asked `archivedOutcome` for each game with the whole game, which the rules check in
  full; it now keeps each game's outcome by id for one revision of the played games
  (`PlayedGames::revision`, bumped by every change), and asks with the five fields the outcome is
  read from when it must. `e2e::history::a_game_saved_again_shows_its_new_result` fails with an
  outcome kept across a change.
- A row reads its game in place; its actions find the game by its id when chosen.
- `openingAt` wrote a FEN after every move. A position is now written out only when its placement
  (the pieces and the side to move) is that of a named position, a filter coarser than the
  table's key, so it never turns a named position away. Tested against the old lookup on every
  seventh named line at each move and on 300 random games; the corpus's 200 recorded answers
  still hold.

`main` (`c05fe2a`) against this change, `KCHESS_PERF_PAGES="Game history,Home"`, 5 alternating
runs, medians (ranges):

| Game history | main | After | Change |
| ------------ | ---: | ----: | -----: |
| First opening | 17.4 ms (17.0–17.8) | 15.3 ms (15.1–15.8) | 1.14× |
| Again | 13.3 ms (12.7–13.5) | 11.2 ms (11.0–11.5) | 1.19× |
| A wheel step | 8.7 ms (8.4–9.0) | 6.7 ms (6.6–7.0) | 1.30× |

What is left is mostly laying the page out again (38% of a wheel step). A part of the page drawn
as a cached view is reused only at the same place in the window, so a scrolled list cannot reuse
its last frame without a change to GPUI's view cache.

### Every page: first openings, Home, and what did not pay

Every sidebar page and Settings profiled (`perf record -g`, `KCHESS_PERF_PAGES` one page at a
time). Over the board, the Analysis board, Puzzles, Practice, Settings, Board editor, Play with
computer, Watch, Tournaments, Insights and Following spend their openings in GPUI drawing the page:
prepaint and layout are 80-95% of each, and no function of KChess's own takes more than a few
percent. What changed:

- **Over the board and the Analysis board are made a moment after the window shows**
  (`shell::prewarm`, 1.5 s after it opens): making them, which reads their sessions and sets up
  the board game's history places, was most of their first openings. Making every page ahead
  (`KCHESS_PERF_PREWARM=all`) costs 1.8 MiB and changes no other page's first opening, whose cost
  is its first frame. Neither page reaches the engine or the network until shown.
- **Home keeps its recent games** until the played games change (`PlayedGames::revision`): it
  sorted every played game on every frame to show six.

`main` (`e743415`) against this change, the whole tour, medians of 3 alternating runs (ranges):

| Opening | main | After | Change |
| ------- | ---: | ----: | -----: |
| Over the board, first | 10.0 ms (9.8–10.1) | 6.3 ms (6.1–6.4) | **1.59×** |
| Analysis board, first | 12.0 ms (11.9–13.4) | 10.5 ms (10.4–11.0) | 1.14× |
| Home, again | 6.6 ms (6.1–7.2) | 5.6 ms (5.6–6.1) | 1.18× |

Tried, measured and taken out:

- **Each group of the sidebar's pages as a cached part of its own.** Every page switch draws the
  whole sidebar again (its key holds the page shown); leaving the sidebar alone would take 1.3 ms
  off a light page's opening (4.1 -> 2.8 ms, an experiment with its highlight left stale). Drawn
  as five keyed parts, it saved 0.3 ms of a switch, but every frame that redraws a page replayed
  five cached parts inside the sidebar's and got slower: a wheel step of Game history 6.9 -> 7.4 ms,
  of Studies 8.4 -> 8.8, of every page about 0.1 ms. Such frames are many more than switches.
- **All openings' families made only in view**, as the Studies cards are. Opening Openings again
  took 13.1 -> 11.4 ms, but a wheel step stayed at 7.4 ms (the page's other parts are most of it),
  and families out of view would be out of reach of Tab and screen readers.

## Looked at and left

**Pages of synced games, newest to oldest.** Game history pages with `LIMIT … OFFSET …`, which
walks every skipped row, so older pages are slower: over 50,000 games, the newest page takes
0.4 ms and the 1,000th 1.8 ms (every account) or 3.2 ms (one account). Keyset paging would make
them flat, but at 3 ms there is nothing a player could notice.

## Reproduce

```bash
cargo test --release -p kchess-core --test store_speed -- --ignored --nocapture
cargo test --release -p kchess-gpui e2e::perf -- --ignored --nocapture
KCHESS_PERF_PAGES=Studies,Home cargo test --release -p kchess-gpui e2e::perf -- --ignored --nocapture
```
