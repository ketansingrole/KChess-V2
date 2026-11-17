# Frame-cost guards

#211 (2026-11-11) made the voice bar ask macOS for the microphone permission on every frame,
about 6 ms each, which took the analysis board's frames from 1.4 to 9 ms
([the fix](2026-11-17-voice-bar-frame-cost.md)). Nothing ran the frame benchmark, so `main` kept it
for nine days. A headless guard now catches that kind of regression on every pull request. A
gate on the frame benchmark catches any other kind, on a Mac.

## On every pull request: no slow call while drawing

The commonest way a frame gets slow is a view that waits on something outside the main thread
while it draws: the system (the microphone permission), the core (`AppCore::request_now`), the
clipboard, a file. Each of those calls says so with `slow_calls::call`
(`apps/gpui/src/slow_calls.rs`). The end-to-end tests draw every frame through `e2e::draw`, inside
`slow_calls::drawing`, which fails the test when the frame made one, naming the call and where it
was made (the slow functions are `#[track_caller]`, so that is the view's line, not theirs).

So every page, panel and dialog the end-to-end tests open, 373 tests, is checked on every
frame they draw, on macOS, Windows and Linux, at no cost in CI time: it is a counter, not a
measurement. A new call that waits outside the main thread marks itself the same way.

Evidence: with the voice bar's line from #211 put back
(`crate::microphone::host(cx).privacy_settings`), 199 of the app's 679 tests fail (the analysis
board 50, local games 29, studies 23, play the computer 22, game history 16, the command palette
14, the page audit 10, puzzles, practice and others), each with

```
a frame waited outside the main thread while drawing; read it before drawing and keep it (slow_calls.rs):
  the system's microphone access (microphone::host) at apps/gpui/src/voice/bar.rs:103:19
```

It cannot see a frame that is slow for another reason (more work in a view, a slower layout).

## Any other frame cost: the frame benchmark on a Mac

`cargo xtask bench` measures real frames, so it sees any cost, but it needs a Mac that draws.
`--fail-above <ratio>` makes it a gate between two builds: it fails when a scenario's median draw
time on the first build is over `ratio` times the last build's and more than 1 ms over it, or its
main thread is over `ratio` times as busy and by more than 10 points, or a scenario drew no frames
or measured nothing on either build. #211 was 6.6× and 7.7 ms on draw time.

```
cargo build --release -p kchess-gpui --features bench && cp target/release/KChess /tmp/KChess-new
# … the same on the base commit, to /tmp/KChess-base
cargo xtask bench --build new=/tmp/KChess-new --build base=/tmp/KChess-base \
  --runs 3 --seconds 15 --scenarios analysis-stepping,board-animation,local-clock,pages-tour \
  --fail-above 2
```

### Why not nightly in CI

The first design ran this each night on `macos-latest`: main against the last passing night,
opening an issue when the gate failed. Its trial run on #245 had #211's line put back in the
measured build, and the gate passed, because the runner's window draws almost nothing.
`analysis-stepping`, `local-clock` and `pages-tour` drew 0 frames a second, and `board-animation`
0.2 (120 on a Mac), with the window counted as visible. Every time was 0 or a handful of samples,
and `draw_ms_p50` was 0 on both builds. The gate now fails on a scenario that drew no frames, so
it can no longer pass such a run. A CI runner cannot guard frame time until it draws frames (a
self-hosted Mac, or a runner with a display). Until then, the headless guard above is CI's
protection, and the benchmark is run by hand before and after work on drawing.
