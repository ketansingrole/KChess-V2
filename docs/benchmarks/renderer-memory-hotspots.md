# Renderer memory hotspot investigation

The packaged desktop study found a renderer RSS increase of roughly 26 MiB at Home and 47 MiB while Analysis was active in the Rust endpoint. This is a process-level observation, not yet a component-level cause. The attribution runner is [renderer-memory.mjs](../../tooling/benchmarks/renderer-memory.mjs); completed results are [WASM instrumentation](raw/renderer-memory-wasm.json) and [GC diagnostics](raw/renderer-memory-gc.json). The original `renderer-memory.json` is preparation metadata, not a completed control run.

The existing seven-pair startup raw data gives this coarse median RSS breakdown:

| Phase and process           | TypeScript |      Rust | Difference |
| --------------------------- | ---------: | --------: | ---------: |
| Home idle — Electron main   |  216.9 MiB | 232.2 MiB |  +15.2 MiB |
| Home idle — renderer        |  156.4 MiB | 182.8 MiB |  +26.4 MiB |
| Home idle — GPU             |   96.8 MiB |  95.8 MiB |   −1.0 MiB |
| Analysis active — renderer  |  232.0 MiB | 279.5 MiB |  +47.4 MiB |
| Analysis active — Stockfish |  385.7 MiB | 386.8 MiB |   +1.1 MiB |

Static inspection found one relevant startup change to measure: the renderer eagerly fetches and instantiates the Rust rules module in the pre-enforced [`rules.client.ts`](../../apps/desktop/app/plugins/rules.client.ts) Nuxt plugin before rendering Home. The checked-out module is 3,920,851 bytes. Its `wasmBinding` also runs a 2,000-input `Math.exp` calibration over two WASM code paths, which is a startup CPU candidate rather than an assumed memory cause. These facts make WASM a testable hypothesis; the module's file size does not predict its linear-memory or compiled-code footprint.

The desktop process tree is therefore the place to investigate; core-worker heap and RSS cannot explain renderer or Electron-main RSS. The existing figures cannot say whether the renderer difference is retained JavaScript objects, V8 heap slack, WebAssembly linear memory, compiled code, allocator reservations, mappings, or Chromium state.

## Renderer attribution probe

Three paired instrumented launches added CDP heap measurements and a pre-document hook that recorded the exported rules `WebAssembly.Memory`. These were three independent launches per version, each with three one-second samples per state; the values below are the median of each launch's three samples, then the median of the three launches (`n=3` independent launches). The hook required a page reload, and the instrumented Home samples reported 48 CDP Documents. This study produced a different Home RSS delta (+59.2 MiB), so it should not revise the original user-visible +26 MiB Home estimate.

| Home after reload                         | TypeScript |      Rust | Rust minus TypeScript |
| ----------------------------------------- | ---------: | --------: | --------------------: |
| Renderer process RSS                      |  173.9 MiB | 233.1 MiB |             +59.2 MiB |
| Renderer V8 used heap                     |   16.0 MiB |  17.6 MiB |              +1.6 MiB |
| Renderer V8 heap capacity                 |   27.1 MiB |  26.4 MiB |              −0.8 MiB |
| Renderer V8 backing storage               |    1.5 MiB |   8.9 MiB |              +7.5 MiB |
| Exported WASM linear memory               |      0 MiB |   9.7 MiB |              +9.7 MiB |
| Electron main RSS (`process.memoryUsage`) |  218.9 MiB | 218.0 MiB |              −0.9 MiB |
| Electron main V8 used heap                |   12.3 MiB |   8.9 MiB |              −3.4 MiB |
| Electron main `external`                  |    4.9 MiB |  41.6 MiB |             +36.7 MiB |

| Analysis with infinite search active      | TypeScript |      Rust | Rust minus TypeScript |
| ----------------------------------------- | ---------: | --------: | --------------------: |
| Renderer process RSS                      |  243.5 MiB | 286.3 MiB |             +42.8 MiB |
| Renderer V8 used heap                     |   23.3 MiB |  22.2 MiB |              −1.1 MiB |
| Renderer V8 heap capacity                 |   36.9 MiB |  33.4 MiB |              −3.5 MiB |
| Renderer V8 backing storage               |    1.5 MiB |   1.4 MiB |             −0.04 MiB |
| Exported WASM linear memory               |      0 MiB |   9.7 MiB |              +9.7 MiB |
| Electron main RSS (`process.memoryUsage`) |  220.8 MiB | 219.5 MiB |              −1.3 MiB |
| Electron main V8 used heap                |   12.8 MiB |  10.3 MiB |              −2.6 MiB |
| Electron main `external`                  |    4.2 MiB |   4.6 MiB |              +0.4 MiB |
| Stockfish child RSS                       |  386.6 MiB | 386.4 MiB |              −0.1 MiB |

In this diagnostic, the renderer's live V8 heap and heap capacity do not explain its larger RSS; both are near parity or lower in Rust. The Rust rules module exposes 9.69 MiB of linear memory in Home and Analysis. CDP backing storage can overlap with that buffer, and its active-state value varied, so the two must not be summed. The main process RSS and its measured heap are also near parity or lower. `process.memoryUsage().external` reports 41.6 MiB for Rust in two of three Home launches and about 4.3 MiB in the third despite similar main RSS, so it is not a stable physical-memory attribution here. `arrayBuffers` was zero in both variants.

The remaining renderer RSS delta is not assigned by these measurements. The WASM module's 3.74 MiB file size and its configured stack do not measure compiled-code pages, and the 9.69 MiB linear buffer is not proof that the same number of physical RSS pages are newly resident. macOS `vmmap` inspection was unavailable without elevated task-port permissions. Full diagnostic samples and the per-launch process data are in [renderer-memory-wasm.json](raw/renderer-memory-wasm.json).

## Forced-GC diagnostic

Three more counterbalanced launch pairs forced renderer collection after the normal samples in each phase. The figures below are the single pre- and post-collection checkpoints from each launch, summarized as median across three independent launches (`n=3` per version); collection timing and these values are diagnostic, not ordinary idle/active measurements.

| Phase and renderer measure              | TypeScript before → after | Rust before → after | Rust minus TypeScript after GC |
| --------------------------------------- | ------------------------: | ------------------: | -----------------------------: |
| Home — V8 used heap                     |            10.5 → 9.0 MiB |      10.9 → 8.8 MiB |                       −0.1 MiB |
| Home — V8 heap capacity                 |            17.1 → 9.6 MiB |      16.8 → 9.6 MiB |                        0.0 MiB |
| Home — renderer RSS                     |         156.8 → 152.4 MiB |   182.4 → 174.2 MiB |                      +21.7 MiB |
| Analysis, engine off — V8 used heap     |           12.7 → 12.2 MiB |     12.4 → 12.0 MiB |                       −0.2 MiB |
| Analysis, engine off — V8 heap capacity |           18.9 → 13.1 MiB |     18.9 → 13.1 MiB |                        0.0 MiB |
| Analysis, engine off — renderer RSS     |         201.6 → 196.2 MiB |   223.6 → 218.7 MiB |                      +22.5 MiB |
| Analysis, active — V8 used heap         |           15.0 → 13.9 MiB |     14.1 → 13.5 MiB |                       −0.3 MiB |
| Analysis, active — V8 heap capacity     |           19.1 → 14.6 MiB |     18.1 → 14.6 MiB |                        0.0 MiB |
| Analysis, active — renderer RSS         |         222.1 → 209.3 MiB |   262.2 → 251.5 MiB |                      +42.2 MiB |

Collection leaves nearly identical V8 capacity and live-heap sizes between builds, while the renderer RSS gap remains about 22 MiB at Home and with Analysis off, and 42 MiB during active Analysis. This rules out retained V8 objects and V8 heap slack as explanations for most of the observed process-RSS difference. GC also returns several MiB of RSS in both builds; those reductions do not identify the remaining mapping. WASM compiled code, allocator/native mappings, and Chromium state remain unmeasured contributors. See [renderer-memory-gc.json](raw/renderer-memory-gc.json) for the raw checkpoints.

The new runner samples three settled points in each fresh packaged launch: Home after the startup settling interval, Analysis with engine analysis off, and Analysis with infinite analysis active. It captures Electron process metrics and descendant RSS, Node `process.memoryUsage()` in Electron main, renderer CDP `Runtime.getHeapUsage` and `Performance.getMetrics`, and `performance.memory` where Chromium exposes it. The runtime categories make different predictions: retained JS objects stay in post-GC heap usage; V8 slack appears in the pre/post collection gap; the rules module's exported `WebAssembly.Memory.buffer.byteLength` measures linear memory; and RSS that remains after those categories are accounted for belongs to other process mappings, compiled code, allocator state, or native components.

`--gc-diagnostic` adds explicit renderer CDP garbage collection after the ordinary samples and records before/after values. Those runs are diagnostic and must be read separately from the ordinary paired study. `--instrument-wasm` adds a pre-document wrapper around `WebAssembly.instantiate` and `instantiateStreaming` that records exported memory sizes. It is a separate diagnostic mode because it patches WebAssembly entry points. The instrumentation reads `memory.buffer.byteLength`; it does not grow memory or retain result objects. The desktop renderer uses the Rust rules WASM binding, while the Electron main process uses the N-API native addon. `process.memoryUsage().external` and `.arrayBuffers` do not account for every Rust/N-API allocation or every WebAssembly memory page, and WASM buffer length excludes compiled code.

Every ordinary run uses a fresh temporary user-data directory, no connected account, an empty `KCHESS_NUXT_URL`, and counterbalanced packaged Rust/TypeScript order. The active phase uses the shared infinite-analysis setting (Hash 128 MB and MultiPV 3); the Stockfish child is reported separately. The runner does not edit the app, product bundle, or generated output. It requires an explicit parent measurement window because engine analysis is CPU-intensive.

## Completed diagnostics

All six launches succeeded in each separate diagnostic mode. The [instrumented raw](raw/renderer-memory-wasm.json) captures one Rust rules linear-memory buffer of 10,158,080 bytes (9.69 MiB), versus no captured WASM buffer in TS. Controlled reload is required to install the instrumentation before instantiation. It shifts Home RSS substantially: instrumented renderer medians were 174.0 MiB TS and 233.1 MiB Rust, unlike the ordinary startup results above. The instrumented run therefore demonstrates linear-buffer capacity, not a numerical decomposition of the original RSS increase. Its CDP used heap changed only about +1.6 MiB at Home and −1.1 MiB during active Analysis. Compiled WASM code remains unmeasured.

The [GC raw](raw/renderer-memory-gc.json) runs without reload and explicitly collects the renderer after each phase. Each cell below is the median of three fresh launches; repeated within-launch samples are not independent replications. Later phases inherit earlier diagnostic collections, so these runs remain separate from ordinary performance measurements.

| Phase                   | TS used V8 heap before → after GC | Rust used V8 heap before → after GC | TS renderer RSS before → after GC | Rust renderer RSS before → after GC |
| ----------------------- | --------------------------------: | ----------------------------------: | --------------------------------: | ----------------------------------: |
| Home                    |                  10.47 → 8.96 MiB |                    10.90 → 8.82 MiB |                 156.8 → 152.4 MiB |                   182.4 → 174.2 MiB |
| Analysis, engine off    |                 12.65 → 12.17 MiB |                   12.39 → 12.00 MiB |                 201.6 → 196.2 MiB |                   223.6 → 218.7 MiB |
| Analysis, engine active |                 15.04 → 13.85 MiB |                   14.11 → 13.53 MiB |                 222.1 → 209.3 MiB |                   262.2 → 251.5 MiB |

Post-GC V8 heap capacity is equal across implementations: 9.56 MiB Home, 13.13 MiB engine-off and 14.63 MiB active. A residual renderer RSS difference of 21.7, 22.5 and 42.2 MiB persists with comparable retained V8 objects and capacity. This is evidence against an additional large retained JS heap explaining these short-session phases, not proof against a long-session leak. It narrows investigation to WASM/native mappings, compiled code, allocator state and Chromium/embedder categories without proving any one cause.

Main `global.gc` was unavailable, so the runner records `mainGc.available=false`; there was no forced main-process collection. Instrumented main `external` reporting varied sharply between launches without a corresponding stable RSS increase, so it is not an allocation attribution. Node RSS and process-tree `ps` RSS are separate measurements and must not be mixed. CDP backing-storage values do not consistently include the tracked WASM buffer; do not add overlapping/incomplete categories to derive total RSS.

The engine child contributes essentially the same memory in both endpoints. The directly observed WASM buffer is a new runtime cost, but its resident pages and compiled code were not isolated. Mapping/allocation profiling and a controlled functional component ablation are the next steps. Do not add forced GC to the app or shrink the 8 MiB WASM stack without preserving supported deep-tree behavior.
