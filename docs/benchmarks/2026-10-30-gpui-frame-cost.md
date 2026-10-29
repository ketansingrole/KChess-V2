# GPUI app: frame cost, idle wake-ups and latency, measured before and after

**Status: complete.** Branch `perf-hill-climb` (local, not pushed), from `a2e89a9` to `1c1bc88`.

## Result

Every frame the app draws now costs less, an idle window no longer wakes 120 times a second, and
a change reaches the screen sooner. Nothing measured got slower, and every scenario draws the
same frames as before. The goal was three times better on every figure; the table marks which
reached it.

| Scenario (median of 5 × 30 s)                       | Figure              | Before |  After | Change |
| --------------------------------------------------- | ------------------- | -----: | -----: | -----: |
| Home, untouched (`home-idle`)                       | Process CPU         |  1.30% |  0.13% | **10×** |
|                                                     | Draw, ms            |   3.62 |   1.18 | **3.1×** |
|                                                     | Change to frame, ms |   7.69 |   1.27 | **6.1×** |
| Shell redrawing 10/s over the openings (`openings-shell`) | Draw p50, ms   |   3.77 |   0.94 | **4.0×** |
|                                                     | Draw p95, ms        |   4.59 |   1.52 | **3.0×** |
|                                                     | Draw time, ms per s |   35.2 |   10.0 | **3.5×** |
|                                                     | Main thread busy    |  3.60% |  1.11% | **3.3×** |
|                                                     | Process CPU         |  6.00% |  2.52% |   2.4× |
|                                                     | Change to frame p50, ms | 7.47 |   1.07 | **7.0×** |
| Timed game over the board (`local-clock`)           | Draw p50, ms        |   5.36 |   2.07 |   2.6× |
|                                                     | Draw time, ms per s |   5.62 |   2.34 |   2.4× |
|                                                     | Process CPU         |  2.60% |  1.03% |   2.5× |
|                                                     | Change to frame p50, ms | 8.93 |   2.13 | **4.2×** |
| Analysis board stepping a line (`board-animation`)  | Draw p50, ms        |   4.79 |   1.83 |   2.6× |
|                                                     | Draw time, ms per s |  105.7 |   46.1 |   2.3× |
|                                                     | Process CPU         | 13.90% |  7.52% |   1.8× |
|                                                     | Change to frame p50, ms | 9.09 |   2.08 | **4.4×** |
| Stepping with the engine on (`analysis-stepping`)   | Draw p50, ms        |   2.00 |   0.61 | **3.3×** |
|                                                     | Draw time, ms per s |   58.6 |   29.3 |   2.0× |
|                                                     | Process CPU         |  8.00% |  4.81% |   1.7× |
|                                                     | Change to frame p50, ms | 7.23 |   5.86 |   1.2× |
| Analysis with the engine on (`analysis-engine`)     | Draw p50, ms        |   2.62 |   1.75 |   1.5× |
|                                                     | Process CPU         |  0.80% |  0.30% |   2.7× |
|                                                     | Change to frame p50, ms | 7.22 |   1.77 | **4.1×** |

Frames drawn per second are unchanged in every scenario (22.3 while the board slides, one a
second for the clock); they are what the window has to show, not a cost. Peak resident memory is
unchanged (130–211 MiB, ±3%). The p95 draw times improve less than the medians (1.2–1.6× outside
the shell scenario): the slowest frames are those that redraw a whole page.

"Change to frame" is new in the harness: the time from a view's first change to the end of the
frame that draws it. Every run's figures are in
[`raw/gpui-frame-cost-2026-10.json`](raw/gpui-frame-cost-2026-10.json).

## What changed

Each step was measured against the one before it (2–3 runs × 15–20 s, so differences under
about 10% are noise); two steps were reverted.

| Change | Where | Effect |
| ------ | ----- | ------ |
| Frames on demand: the display link runs only for input and animations; a lone change is drawn at once | `vendor/gpui-pre-macos` (`FrameDemand`) | Idle CPU 1.3% → 0.13%; change to frame 7–9 ms → 1–2 ms |
| Cached views inside a redrawn cached view are reused, but only ranges from the last frame | `vendor/gpui-pre/src/view.rs` | Board slides: draw time −35% |
| Replayed primitives keep their orders, shifted together, instead of being inserted into the bounds tree again | `vendor/gpui-pre/src/scene.rs`, `bounds_tree.rs` | Openings under shell redraws: draw time −36% |
| Tab order sorted when Tab asks, not on every insert | `vendor/gpui-pre/src/tab_stop.rs` | Openings under shell redraws: draw p50 −12% |
| `Part`: a slice of a view's render drawn as a cached view; keyed parts redraw only when what they show changed | `apps/gpui/src/part.rs` | — |
| Sidebar, top bar and sidebar toggle as keyed parts | `apps/gpui/src/shell.rs` | Shell redraws −40% |
| Board pages' panels as parts; the local page's clock faces as parts its ticker redraws alone | `pages/analysis`, `pages/local.rs` | With the steps above: clock ticks 5.8 → 3.3 ms of drawing per second |
| Rules calls from Rust pass values, not JSON text | `kchess-domain::api::call_value`, `kchess-games::rules` | The clock's ticker no longer writes and reads the game as JSON text 3–4 times every 100 ms (not measured on its own) |

Reverted:

- **Display link restarted for every change.** It drew every change 13–15 ms after it happened
  (6–9 ms before), and each restart spawns a thread. Replaced by drawing a lone change at once.
- **Analysis panel split in two** (engine lines apart from the tools), so an engine update would
  redraw only its lines. Engine updates drew 27% faster, but stepping through a game with the
  engine on drew 43% slower and took 6.1 ms from change to frame (5.5 ms before). Reverted.

### Why not three times on every figure

- **A small change redraws every view around it.** GPUI marks a notified view and every view
  above it dirty, so a clock face, a sliding piece or an engine line still re-renders and lays out
  the page and the shell around it (now without the cached parts inside them). Clock ticks, board
  slides and engine updates are 1.5–2.6× cheaper; three times would need GPUI to redraw a view
  inside ancestors that it otherwise reuses, splicing it into their replayed frame (dispatch tree,
  element state, hitboxes and paint order). That is a follow-up of its own.
- **Process CPU while animating** includes Metal's and Core Animation's threads, which work for
  every frame shown and did not change. On the main thread board slides cost 2.3× less.
- **Memory.** Nothing here was meant to reduce it, and it did not change.

## Signed in to Lichess, and every page opened

Added after the record above: `cargo xtask bench` now runs a stand-in Lichess on 127.0.0.1
(`tooling/xtask/src/fake_lichess.rs`) that the app signs in to through its own OAuth flow, in a
first launch, before the measured one. `pages-tour` opens every sidebar page and Settings in
turn: each page once ("first", the page is made), then again and again ("again"). Before is
`main` with this bench, after is `62c707e`; median of 3 × 20 s.

| Scenario | Figure | Before | After | Change |
| -------- | ------ | -----: | ----: | -----: |
| Play on Lichess, signed in (`lichess-idle`) | Process CPU | 1.0% | 0.14% | **7.3×** |
| | Change to frame, ms | 12.3 | 4.3 | 2.9× |
| A live Lichess game (`lichess-game`) | Draw p50, ms | 4.13 | 1.88 | 2.2× |
| | Draw time, ms per s | 36.7 | 17.9 | 2.1× |
| | Change to frame p50, ms | 8.29 | 2.73 | **3.0×** |
| Lichess TV (`lichess-tv`) | Draw p50, ms | 4.85 | 1.56 | **3.1×** |
| | Draw time, ms per s | 47.0 | 13.9 | **3.4×** |
| | Change to frame p50, ms | 7.90 | 1.86 | **4.3×** |
| Every page (`pages-tour`) | Opening again, p50, ms | 10.0 | 4.9 | 2.1× |
| | Opening again, p95, ms | 17.0 | 6.7 | 2.5× |

Opening a page again is 1.5–9× faster depending on the page (Puzzles 1.5×, Settings 1.6×,
Practice 1.9×, Watch 2.5×, Game history 9×). Opening one the first time is 1–15× faster: Home,
Watch, Game history and Lichess insights most, the board pages 1.1–1.4×, Settings unchanged,
and Openings 0.9× (21.1 → 23.4 ms, in two separate comparisons): the one figure that got worse.

What changed for these:

- **Shaped text is kept** while it is off screen (`vendor/gpui-pre`, `LineLayoutCache::retained`):
  a page opened again no longer shapes all its text with CoreText.
- **The live game's and TV's panels, TV's channel list and its clock faces are cached parts**
  (`pages/online/game.rs`, `pages/watch`): a sliding move redraws the page around the board
  alone, and TV's clock tick redraws the face whose text changed instead of the page twice a
  second (so TV draws one frame a second fewer).

A first opening is mostly work done once per launch: rasterizing glyphs and icons, a board's
pieces at its size (on the main thread), shaping text for the first time, and uploading to the
GPU atlas (sampled with `sample` over five launches).

## Correctness

Caching more means more chances to show something stale, so the checks were tightened first:

- **The end-to-end tests draw as the window does** (`apps/gpui/src/e2e/mod.rs`): only views that
  changed, cached ones reused, after delivering next-frame callbacks. GPUI Kit's `render_frame`
  redraws every view, which hid stale caches. On an early state of this branch it caught a panic
  (cached views reusing ranges of an older frame) that the old harness passed.
- **Reused frames are pixel-identical** (`e2e::screens`, macOS, needs a GPU): every page at three
  sizes drawn afresh and drawn again from its cached frame must match byte for byte.
- `e2e::shell::tab_and_shift_tab_walk_the_same_controls` (tab order),
  `e2e::local::a_timed_games_clock_runs_for_the_side_to_move` (clock faces redraw on their own),
  and the rules corpus answered through `call_value` as through `call` (32k calls).
- Each new check was confirmed to fail against a broken version of the code it guards.

## Method

- **Harness**: `apps/gpui/src/bench.rs` (`--features bench`), as in the
  [render caching study](2026-10-27-gpui-render-caching.md), now also reporting the change-to-frame
  latency, how much of the run the window was visible (a covered run is left out), and a sixth
  scenario, `analysis-stepping`.
- **Runner**: `cargo xtask bench` (the Node runner left with Node). Each run gets a fresh
  throwaway profile; builds alternate run by run, odd runs in reverse order; medians.
- **Builds**: release, `cargo build --release -p kchess-gpui --features bench`. "Before" is
  `a2e89a9` (`main`) with this branch's `bench.rs`; "after" is `1c1bc88`.
- **Host**: Apple M5, 10 logical CPUs, 32 GiB, macOS 27, built-in ProMotion display, on mains
  power; nothing else ran during the final measurement.

## Reproduce

```bash
cargo build --release -p kchess-gpui --features bench && cp target/release/KChess /tmp/KChess-after
git worktree add /tmp/kchess-before a2e89a9 && cp apps/gpui/src/bench.rs /tmp/kchess-before/apps/gpui/src/
(cd /tmp/kchess-before && cargo build --release -p kchess-gpui --features bench)
cargo xtask bench --build before=/tmp/kchess-before/target/release/KChess \
  --build after=/tmp/KChess-after --runs 5 --seconds 30 --out results.json
```
