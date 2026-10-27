# GPUI app: render caching and clock ticks, measured before and after

**Status: complete.** Branch `gpui-render-caching` (local, not pushed), from `955567c` (benchmark
harness only) to `218aa77`.

## Result

A running game clock now costs a third of the CPU it did, and redraws that only the shell needed
no longer rebuild the page. Nothing measured got slower. One change that made slides smoother was
measured, found to cost more CPU, and reverted.

| Scenario (median of 5 × 30 s)  | Metric              | Before |  After | Change      |
| ------------------------------ | ------------------- | -----: | -----: | ----------- |
| Timed game over the board      | Frames drawn per s  |    9.6 |    1.1 | **−89%**    |
| (`local-clock`)                | Draw time, ms per s |   46.9 |    5.4 | **−88%**    |
|                                | Process CPU         |  7.24% |  2.40% | **−67%**    |
|                                | Draw p50, ms        |   4.99 |   5.31 | +6%         |
| Shell redrawing at 10/s over   | Draw p50, ms        |  12.78 |   3.38 | **−74%**    |
| the openings grid              | Draw p95, ms        |  13.92 |   4.33 | **−69%**    |
| (`openings-shell`)             | Draw time, ms per s |  120.1 |   32.0 | **−73%**    |
|                                | Process CPU         | 14.18% |  5.34% | **−62%**    |
| Analysis board stepping a line | Draw p50, ms        |   4.78 |   4.61 | −3%         |
| (`board-animation`)            | Process CPU         | 13.75% | 13.51% | −2% (noise) |
| Analysis with the engine on    | Draw time, ms per s |   1.94 |   1.83 | −6% (noise) |
| Home, untouched (`home-idle`)  | Process CPU         |  1.02% |  1.04% | none        |

Run-to-run ranges for the two large changes don't overlap. Local clock CPU was 6.83–7.28%
before and 2.20–2.49% after. Openings draw time was 119.9–120.8 ms/s before and 31.9–32.4 ms/s
after. Peak resident memory is unchanged in every scenario (±0.5%). Per-run values are in
[`raw/gpui-render-caching-2026-10.json`](raw/gpui-render-caching-2026-10.json).

Home drew one frame in each 30 s run after the change (none before), about 3 ms each. The
source was not identified. It costs about 0.1 ms per second.

## What changed, and what each part did

| Step | Commit                           | Change                                                                                          | Measured effect                                                                                                                                                                        |
| ---- | -------------------------------- | ----------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 3    | `7ebd309`                        | Views that read settings, preferences and shared models are redrawn when they change.           | Correctness prerequisite for caching. No timing effect.                                                                                                                                |
| 4    | `5663419`                        | Page, boards and card pictures embedded as cached views (`Entity::cached` / `AnyView::cached`). | `openings-shell`: draw time −71%, CPU −60% (3 × 20 s against baseline). Other scenarios: within ±10%, mostly noise.                                                                    |
| 5    | `bbaa2be`, reverted in `218aa77` | Board slides driven by `request_animation_frame` instead of a 16 ms timer.                      | `board-animation`: 82% more frames (smoother slides at the display's refresh), draw p50 −31%, but CPU **+27%**. Reverted, per the plan's rule that a step which regresses is reverted. |
| 6    | `1d84f60`                        | Clock tickers redraw only when a shown clock face (or an online countdown) changes.             | `local-clock`: frames −89%, CPU −67% (3 × 20 s against step 5).                                                                                                                        |
| 7    | `3401b29`                        | `StatusBanner` and `StudyCard` as `RenderOnce` components.                                      | Neutral, as expected: `openings-shell` −6%, `local-clock` +3% (both noise).                                                                                                            |

### Why the board cache did almost nothing, and the page cache did a lot

GPUI marks a notified view **and every view above it** dirty (`Window::mark_view_dirty`). So
when a clock or an engine line notifies the page, the page and the shell re-render no matter what
is cached under them. Only cached _children_ are reused, and the board is cheap to draw. It
never appeared above the profiler's 0.5 ms per-view floor. In `local-clock` the per-tick cost
(~5 ms) is the page's own render (~1.4 ms), the shell's (~0.6 ms), and layout and paint of the
whole tree. With the board cached, its render moves from the layout phase into prepaint (where
GPUI renders cached views); the total is the same.

Two kinds of change pay off:

- **Draw fewer frames.** A clock at 100 ms ticks shows a new value once a second, except for
  tenths in the last ten seconds. Comparing the shown text before notifying removes 9 of 10
  frames (step 6). The earlier idea of moving clocks into their own small views would not have
  helped, because a clock view's notify still dirties the page above it. It was dropped.
- **Skip whole subtrees whose parent alone changed.** The page sits under the shell. When only
  the shell redraws (sidebar, command palette, network/account/update state), the cached page
  is reused (step 4). That is the `openings-shell` result.

### Caching needed invalidation work first

A cached view is redrawn only when it, or a view inside it, is notified. An entity it merely
_reads_ while rendering does not mark it dirty; it only asks the window for a frame. The app
read settings (34 files), view preferences, and shared models (accounts, online, studies, played
games, reviews, puzzle database, voice, network, updates) in `render` without observing them.
That only worked because every frame re-rendered everything. Step 3 made this explicit:

- Settings changes call `window.refresh()` (`shell.rs`), and preference writes call
  `cx.refresh_windows()` (`prefs.rs`). A refresh ignores caches. Both are user actions, so this
  costs nothing measurable.
- The shell observes the shared models and notifies the page as well as itself
  (`Workspace::shared_changed`).
- Rush no longer receives Lichess's Storm panel as an element handed in each frame. It holds
  the stats view, renders the panel itself and observes it.
- Four mutations that skipped `notify` now notify: repertoire misses, the online challenge
  form's rated flag (twice), and the Practice page's `shown`.
- Regression: `prefs::tests::a_changed_preference_reaches_cached_views`, a headless GPUI test.
  It fails without the refresh: the cached view keeps showing 0 instead of 5.

Not cached, and why:

- **The position explorer and the charts.** A cached view is laid out from the style it is given,
  not measured from its contents. The explorer's height depends on its contents. The charts
  rarely redraw, so caching them has nothing to save.
- Render-time effects in Puzzles (`shown(...)` for the history and stats tabs) and Home
  (`load(...)` of Lichess activity) were left in place. They are guarded by tab and account
  comparisons, and the redraw they need is now forced by the preference refresh or the shell's
  model forwarding.

## Method

- **Harness** (`apps/gpui/src/bench.rs`, compiled only with `--features bench`, which turns on
  GPUI's profiler). `KCHESS_BENCH=<scenario>` drives the real window through the pages' own
  methods; no input is sent to the desktop. After a 5 s warm-up it drains GPUI's foreground
  journal for the measured window and records:
  - every draw's duration and phases;
  - each view's self time (above GPUI's 0.5 ms floor);
  - foreground busy time;
  - the process's CPU time (`getrusage`, all threads; the engine runs in its own process and
    is not counted);
  - peak RSS.

  It writes JSON and quits.

- **Scenarios**:
  - `home-idle`: control.
  - `local-clock`: a 5+3 game over the board after 1.e4, clock running.
  - `board-animation`: the analysis board stepping a 24-ply Ruy Lopez line, one move every
    400 ms, engine off.
  - `analysis-engine`: infinite analysis of the start position.
  - `openings-shell`: the openings grid while the shell is notified 10 times a second, as its
    observers do.
- **Runner** (`tooling/gpui-render-benchmark.mjs`): each run gets a fresh throwaway profile
  (`KCHESS_USER_DATA_DIR`). Builds alternate run by run (odd runs reverse the order), and the
  report gives medians.
  - Final comparison: 5 runs × 30 s per scenario.
  - Per-step attribution: 3 runs × 20 s.
  - Nothing else was compiled or run during measurement. One early baseline run that overlapped a
    compile was discarded.
- **Builds**: release, `cargo build --release -p kchess-gpui --features bench`, rebuilt at each
  commit and copied aside (the shared Cargo target can hold another checkout's binary). "Before"
  is `955567c`, which is `a517272` plus the harness only.
  - "After" for `board-animation` is `218aa77`.
  - For the other scenarios it is `3401b29`, which still contained step 5. Step 5 only changes
    how slides are timed, and these scenarios have no slides during measurement.
- **Host**: Apple M5, 10 logical CPUs, 32 GiB RAM, macOS 27.0.1, built-in Retina display, on
  mains power.
- **Visual check**: one capture of the final build's window during `local-clock`. The cached
  board is square in its column, the page fills its slot, the clock counts down, and the
  `StatusBanner` renders.

## What this does not show

- **Frame timing with real input**: drags, typing and scrolling were not driven. Scenarios use
  the pages' own methods, which skip hit-testing and input dispatch.
- **Online games**: the online ticker got the same change as the local one (redraw when a clock
  face or an abort/claim countdown changes), but an online game needs a Lichess account and
  stream fixtures, which the harness does not set up.
- **The computer game** got the same change and is not benchmarked either. Its clock path is the
  same shape as the local game's.
- **GPU work**: the profiler times CPU-side draw phases and submission, not GPU execution.
- **Other platforms**: macOS only.

## Follow-ups

- **Smoother slides at less cost.** `request_animation_frame` gave visibly more frames per slide
  for 27% more CPU while sliding. Capping it at 60 Hz, or animating with GPUI's `with_animation`
  on the sliding pieces only, might keep the smoothness at today's cost. This needs its own
  measurement.
- **Cheaper page renders.** Every remaining tick still re-renders the page (~1.4 ms on the
  local page) and the shell. Profiling `LocalPage::render` and `AnalysisPage::render` is the
  next lever.

## Dependency note

Headless GPUI tests need GPUI Kit's `test-support` feature as a dev-dependency. That only resolves
with `unicode-properties` 0.1.3, which the lockfile now pins (was 0.1.4; patch release of a
Unicode table crate used by GPUI's web platform). The lockfile also gains test-only graphics
crates. Normal builds don't compile them.

## Reproduce

```bash
cargo build --release -p kchess-gpui --features bench
cp target/release/KChess /tmp/KChess-after
git checkout 955567c && cargo build --release -p kchess-gpui --features bench
cp target/release/KChess /tmp/KChess-before && git checkout -
node tooling/gpui-render-benchmark.mjs --build before=/tmp/KChess-before \
  --build after=/tmp/KChess-after --runs 5 --seconds 30 --out results.json
```

`analysis-engine` uses the Stockfish at `target/gpui-package/stockfish/stockfish`
(`node tooling/gpui-package.mjs` fetches it).
