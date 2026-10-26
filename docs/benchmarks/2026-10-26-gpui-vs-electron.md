# GPUI app vs Electron app: launch, resources, size and engine

**Status: complete for what can be measured without driving the UI.** Both apps were built from
the same commit and measured from outside the process the same way.

## Result

On this Mac, the GPUI app beats the Electron app on every resource metric measured and finishes
drawing Home a little sooner. Electron puts a window on screen about 35 ms earlier, but that window
is still blank.

| Metric (p50, reused profile unless noted)                    |      Electron |          GPUI | GPUI vs Electron      |
| ------------------------------------------------------------ | ------------: | ------------: | --------------------- |
| Window on screen                                             |        268 ms |        305 ms | 37 ms later           |
| Home visually complete                                       |        433 ms |      ≤ 412 ms | ≥ 21 ms sooner (≥ 5%) |
| Home visually complete, fresh profile                        |        447 ms |      ≤ 405 ms | ≥ 42 ms sooner (≥ 9%) |
| CPU time used in the first 15 s                              |        0.89 s |        0.36 s | 2.5× less             |
| Physical memory footprint at idle (`phys_footprint`, summed) |       272 MiB |        92 MiB | 3.0× less             |
| Resident memory at idle (RSS, summed)                        |       519 MiB |       116 MiB | 4.5× less             |
| Processes / threads                                          |       4 / 102 |        1 / 18 | —                     |
| Idle CPU, average over 30 s on Home                          |         1.16% |         0.77% | 34% less              |
| Energy Impact at idle (`top` POWER)                          |           4.1 |           3.9 | 5% less (fresh: 18%)  |
| Exit after SIGTERM                                           |        178 ms |         77 ms | 2.3× sooner           |
| App bundle, logical bytes                                    |      355.3 MB |      156.7 MB | 56% smaller           |
| App bundle without the chess engine                          |      353.6 MB |       51.8 MB | 85% smaller           |
| ZIP (`ditto -c -k --sequesterRsrc --keepParent`)             |      143.3 MB |      101.7 MB | 29% smaller           |
| ZIP without the chess engine                                 |     ~142.6 MB |       20.2 MB | 86% smaller           |
| Engine search speed, 1 / 4 threads (startpos, nodes/s)       | 1.35M / 5.05M | 1.64M / 6.20M | +22% / +23%           |
| Engine peak RSS during search, 1 / 4 threads                 | 263 / 338 MiB | 362 / 444 MiB | ~100 MiB more         |

"≤" for GPUI: its first captured frame was already the finished Home page (see Method), so the
true time is at or before the figure shown.

The engine rows compare the engines each app ships, not the two UI frameworks. Electron runs
Stockfish 19 **lite** (1.7 MB WebAssembly, small network) under Node. GPUI ships native
Stockfish 19 (100 MB, full network). The native engine searches 13–49% more nodes per second and
reaches equal or greater depth in the same time. It also uses the stronger network, which this
study does not measure, and costs ~100 MiB more memory while searching and 100 MB more download.
The engine accounts for two-thirds of the GPUI bundle.

## What this does not show

- **Interaction**: page navigation, typing and drag latency, scrolling and animation frame rates,
  and analysis running inside the app were not measured. GPUI cannot be driven by Playwright, and
  synthetic input outside the test harness is unsafe on a shared desktop (it once typed into
  another app). Measuring these needs in-app timing hooks in both apps.
- **Feature parity**: the GPUI app is a preview with open items in `apps/gpui/PARITY.md`. Lower
  resource use with fewer features does not by itself mean the GPUI app is the better product.
- **Other platforms**, long sessions (leaks), energy under load, and signed/notarized packages.
- Exit timing: Electron runs its quit preparation on SIGTERM; GPUI's handling differs, so treat
  this row as indicative.
- Idle Energy Impact is noisy (Electron 2.8–5.2, GPUI 3.5–4.5 across launches).

## Compared builds and host

Both apps were built from `a5a46d3` (`main`, clean tree) after forcing a native rebuild (all
`crates/**/src/*.rs` touched) so no other worktree's library was packaged:

- Electron: `pnpm run pack:mac` → `dist/mac-arm64/KChess.app` (Electron 44.9.0). SHA-256:
  `KChess` `71bf2b1a…`, `Electron Framework` `dff15be7…`, `app.asar` `520c5b3a…`.
- GPUI: `node tooling/gpui-package.mjs` → `target/gpui-package/stage/KChess Preview.app`. SHA-256:
  `kchess` `3a68f318…`, `stockfish` `40b1692e…`.

Host: macOS 27.0.1, Apple M5, 10 logical CPUs, 32 GiB RAM. Both builds signed ad hoc, launched
directly by executable path, no account connected, default settings (both check for updates at
startup).

## Method

Scripts: `tooling/benchmarks/gpui-electron/`. Raw data:
[`raw/gpui-vs-electron-launch-2026-10.json`](raw/gpui-vs-electron-launch-2026-10.json) and
[`raw/gpui-vs-electron-engine-2026-10.json`](raw/gpui-vs-electron-engine-2026-10.json).

**Launches** (`launch.mjs`): one untimed warm-up each, then 7 alternating pairs (odd pairs
Electron first, even pairs GPUI first) with a new profile per launch (`KCHESS_USER_DATA_DIR` in a
temporary directory), then 7 more pairs reusing one profile per app (created by one uncounted
launch). 30 timed launches in total.

- **Window on screen**: `pshot.swift` starts the executable and polls `CGWindowList` every ~4 ms
  for an on-screen, layer-0 window wider than 300 px owned by the PID.
- **Visually complete**: the probe then captures that window with ScreenCaptureKit for 5 s (one
  frame every ~29 ms, reduced to a 96×60 grayscale thumbnail). Visually complete is the earliest
  frame after which every frame matches the final one (under 0.5% of pixels changed by more than
  12 levels). The first capture arrives about 105 ms after the window appears. For GPUI that
  frame was already complete in all 30 launches, so its figure is an upper bound; Electron's first
  frame was partial and completed ~40 ms later. An accessibility-tree probe was tried first and
  dropped: enabling Chromium's accessibility tree pushed Electron's ready time to ~2.2 s, which
  measured the probe, not the app.
- **Process tree**: every descendant of the launched PID (`ps`). At 15 s: RSS, `footprint -p`
  (`phys_footprint`, the figure Activity Monitor's Memory column uses), and threads (`ps -M`).
  Neither app had a Stockfish process at idle on Home.
- **CPU**: cumulative CPU time of the tree at 15 s (startup work), and the delta over the next
  30 s (idle). **Energy Impact**: last of three 5 s `top -stats power` samples, summed over the tree.
- **Exit**: SIGTERM to the main PID, then time until the whole tree is gone.

Memory by process, reused profile (p50, RSS / footprint MiB): Electron main 221 / 90, renderer
166 / 73, GPU and network helpers 132 / 109; GPUI single process 116 / 92. RSS sums double-count
shared framework pages, which is why `phys_footprint` is the fairer whole-app figure.

**Sizes**: logical bytes of every file in each bundle, and a ZIP made with the flags the GPUI
packager uses. "Without the engine" removes GPUI's `stockfish`, and subtracts Electron's 1.7 MB
`stockfish-19-lite.wasm` (approximate for the Electron ZIP).

**Engine** (`engine.mjs`): the same UCI session against each shipped engine, launched as the app
launches it (Electron's binary with `ELECTRON_RUN_AS_NODE=1` running `stockfish-19-lite.js`; the
native `stockfish` directly). `Hash 64`, `ucinewgame` per position, `go movetime 3000` on 4
positions at 1 and 4 threads, two alternating repetitions; nodes/s and depth from the last `info`
line, peak RSS sampled every 200 ms.

| Position   | Threads | Electron nodes/s | GPUI nodes/s | Difference | Depth (E / G) |
| ---------- | ------: | ---------------: | -----------: | ---------: | ------------: |
| Start      |       1 |            1.35M |        1.64M |       +22% |       26 / 29 |
| Italian    |       1 |            1.38M |        1.56M |       +13% |       30 / 31 |
| Middlegame |       1 |            1.20M |        1.49M |       +24% |       21 / 21 |
| Endgame    |       1 |            2.12M |        3.16M |       +49% |       46 / 68 |
| Start      |       4 |            5.05M |        6.20M |       +23% |       28 / 30 |
| Italian    |       4 |            5.18M |        6.03M |       +16% |       31 / 31 |
| Middlegame |       4 |            4.50M |        5.42M |       +20% |       20 / 21 |
| Endgame    |       4 |            9.53M |       12.58M |       +32% |       60 / 64 |

## Launch detail

| p50 (IQR)              | Electron fresh   | GPUI fresh       | Electron reused  | GPUI reused      |
| ---------------------- | ---------------- | ---------------- | ---------------- | ---------------- |
| Window on screen, ms   | 271 (259–276)    | 297 (296–302)    | 268 (266–270)    | 305 (300–308)    |
| Visually complete, ms  | 447 (441–454)    | ≤ 405 (402–410)  | 433 (432–437)    | ≤ 412 (405–414)  |
| CPU in first 15 s, s   | 0.93 (0.93–0.94) | 0.36 (0.36–0.38) | 0.89 (0.89–0.89) | 0.36 (0.36–0.36) |
| Footprint, MiB         | 273 (271–275)    | 93 (93–93)       | 272 (272–273)    | 92 (92–93)       |
| RSS, MiB               | 535 (534–536)    | 117 (116–117)    | 519 (518–520)    | 116 (116–116)    |
| Threads                | 102 (100–106)    | 17 (17–18)       | 102 (101–102)    | 18 (17–19)       |
| Idle CPU, %            | 1.16 (1.15–1.18) | 0.77 (0.75–0.79) | 1.16 (1.16–1.20) | 0.77 (0.75–0.77) |
| Energy Impact          | 4.5 (4.4–4.6)    | 3.7 (3.7–3.8)    | 4.1 (4.1–4.3)    | 3.9 (3.7–3.9)    |
| Exit after SIGTERM, ms | 177 (173–180)    | 78 (76–81)       | 178 (173–179)    | 77 (74–79)       |

## Reproduce

```bash
pnpm run pack:mac
node tooling/gpui-package.mjs
node tooling/benchmarks/gpui-electron/launch.mjs 7
node tooling/benchmarks/gpui-electron/summary.mjs perf/results/gpui-electron-launch.json
node tooling/benchmarks/gpui-electron/engine.mjs
```

The launch run opens and closes about 32 windows over ~25 minutes and needs the host to have
Screen Recording permission. It sends no input to either app.
