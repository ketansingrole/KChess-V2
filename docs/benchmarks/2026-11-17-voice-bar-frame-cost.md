# The voice bar asked macOS about the microphone on every frame (2026-11-17)

**Status: fixed.** Board pages drew a frame in about 9 ms instead of 1.4 ms on `main` from #211
(2026-11-11) on. It never shipped: #211 came after `v2026.11.7`.

## Cause

#211 moved the microphone's state behind `microphone::host`, so tests can pin it. The voice bar
called it while drawing, to show the Privacy settings link, which is a constant for each
platform; `host` also asks the system whether the app may record. On macOS that is
`AVCaptureDevice.authorizationStatus`, a round trip to the privacy service: **6.2 ms a call**
here (20 ms the first time), measured with a loop of 200 calls. Every page with a voice bar
(the analysis board, the game pages, the trainers) paid it on every frame that drew its bar.

The time is spent waiting, not computing: on `main` the main thread was 91% busy in
`analysis-engine` while the process used 20% of a core, and `board-animation` dropped frames
(35 a second where it now draws 42).

## Bisect

`git bisect --first-parent` over the 127 merges in `353c18c4..39634bea`, release builds with
`--features bench`, `analysis-stepping` for 10 s, bad above a 4 ms draw p50. Draw p50 at each
step: `186c2eb4` 1.43, `7a2748c9` 1.36, `cb1c7bbf` 8.94, `42982906` 8.93, `1e801469` 8.86,
`3843b2e1` 1.35. First bad: `1e801469`, #211. #117 (slides on the display's frame clock) was
checked first and is not it: it draws more frames during a slide, as intended, each as cheap as
before.

## Fix

The bar reads `microphone::privacy_settings`, which never asks the system. `host` is read when
Settings shows Voice input or the window comes back, and its comment says it is not for
drawing. Tests count `host` reads; `stepping_through_moves_never_asks_the_system_about_the_microphone`
(`e2e/analysis.rs`) steps through a game and requires none (31 before the fix).

`local-clock` drew its clock 0.05 times a second because the scenario broke, not the app: since
#223, `replace_game` waits until the saved game is read, and the bench played its first move
before that, so it was refused and no clock started. The bench now does both once the game is
read.

## Figures

`cargo xtask bench`, `main` at `dab85ab0` against the fix, 3 runs of 20 s each, alternating;
medians, an Apple Silicon Mac at 120 Hz. 11-06 is `353c18c4` from
[the motion bench](2026-11-06-motion.md).

| Scenario            | Figure              | 11-06 |  main |   fix |
| ------------------- | ------------------- | ----: | ----: | ----: |
| `analysis-stepping` | Draw p50, ms        |  1.40 |  9.15 |  1.38 |
|                     | Frames per second   |   120 |    99 |   120 |
|                     | Main thread busy    |   17% |   91% |   17% |
| `analysis-engine`   | Draw p50, ms        |     – |  9.09 |  1.25 |
|                     | Frames per second   |     – |   100 |   120 |
|                     | Main thread busy    |     – |   91% |   15% |
| `board-animation`   | Draw p50, ms        |  1.50 |  6.68 |  1.72 |
|                     | Draw p95, ms        |     – | 19.12 |  2.86 |
|                     | Frames per second   |    23 |    35 |    42 |
| `local-clock`       | Draws per second    |   1.1 |  0.05 |  1.09 |
|                     | Draw p50, ms        |  1.61 |     – |  2.22 |
| `pages-tour`        | Draw p50, ms        |     – |  8.18 |  0.95 |
|                     | Page open p95, ms   |     – |  32.0 |  16.0 |

`board-animation` draws more frames than on 11-06 because of #117. `local-clock`'s main draw p50
is its one draw in the run, not a clock. The other scenarios (home, openings, the Lichess pages)
show no voice bar and did not change.
