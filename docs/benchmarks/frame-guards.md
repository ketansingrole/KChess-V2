# Frame-cost guards

#211 (2026-11-11) made the voice bar ask macOS for the microphone permission on every frame,
about 6 ms each, which took the analysis board's frames from 1.4 to 9 ms
([the fix](2026-11-17-voice-bar-frame-cost.md)). Nothing ran the frame benchmark, so `main` kept it
for nine days. Two guards now watch for it, a cheap exact one on every pull request and a
measured one each night.

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

## Each night: the frame benchmark on macOS

`.github/workflows/bench.yml` builds `main` and the commit the last passing night measured (the
first night: the commit before), both without fat LTO, and runs `cargo xtask bench` on a macOS
runner with `analysis-stepping`, `board-animation`, `local-clock` and `pages-tour`: three runs of
15 s each, the builds alternating run by run, so a change in the runner's speed falls on both.
`--fail-above 2` fails the job when a scenario's median draw time is over twice the base's and
more than 1 ms over it, or its main thread is over twice as busy and by more than 10 points, or a
scenario measured nothing on one of the builds. #211 was 6.6× and 7.7 ms on draw time.

A failed night opens the issue "Frames cost more on main" (or comments on it if it is open), with
the figures; the run's summary and its `bench` artifact hold them all. The base moves only on a
passing night, so a regression fails every night until it is fixed. The repository is public, so
the macOS runner costs nothing; the job takes about N minutes, mostly the two builds.

A pull request that changes the benchmark or the workflow runs it against its base. Both builds
must know every scenario in the workflow's `SCENARIOS`, so add a new scenario there only once it is
on `main` (a base without it writes no report, and the job fails). Run it by hand
against any commit with `gh workflow run bench.yml -f base=<commit>`.
