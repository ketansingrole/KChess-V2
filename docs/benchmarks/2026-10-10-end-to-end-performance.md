# KChess end-to-end performance comparison

**Status: final core, desktop, CPU-quiescence, and normalized package-size reruns are complete.**

## Summary for users

The migration results are mixed. Follow-up Rust fixes cut warm 50k-game Insights time from about 1.42 seconds to 0.72 seconds and made the warm history page 0.38 ms versus 0.25 ms in TypeScript. Packaged startup and navigation are effectively unchanged. Whole-app RSS is about 8% higher at idle and 4% higher during analysis, but the Rust engine-off action exits Stockfish in all seven trials, cutting RSS by 343 MiB compared with TypeScript retaining the engine; re-enabling analysis then costs about 162 ms for the process restart. The normalized package comparison shows a 2.01% larger full app and 3.20% larger unsigned ZIP size proxy. These are implementation and resource-policy changes, not Stockfish search or playing-strength improvements.

| Surface or workload                                                         |           TypeScript baseline |          Initial Rust endpoint |           Final Rust endpoint |
| --------------------------------------------------------------------------- | ----------------------------: | -----------------------------: | ----------------------------: |
| Visible Start / Play enabled                                                |                  417 / 430 ms |                   426 / 439 ms |                  419 / 433 ms |
| Analysis / History navigation                                               |                    84 / 46 ms |                     87 / 51 ms |                    84 / 45 ms |
| Core module import / empty-profile create+load                              |                23.1 / 4.49 ms |                 7.80 / 3.90 ms |                 8.1 / 4.02 ms |
| 50k-game insights, warm                                                     |                    1,418.8 ms |                       1,399 ms |                      717.5 ms |
| 50k-game history page, warm                                                 |                      0.247 ms |                       1.098 ms |                      0.383 ms |
| 50k-game overview, warm                                                     |                       9.75 ms |                       10.90 ms |                      10.99 ms |
| Core worker max RSS, fixture included                                       |                     254.4 MiB |                      144.0 MiB |                     147.7 MiB |
| Whole-app RSS, Home idle / analysis active / 1 s after turning analysis off | 520.7 / 1,029.7 / 1,042.5 MiB | 559.5 / 1,073.5 / not measured |   562.1 / 1,073.8 / 699.3 MiB |
| Engine re-enable to first depth update                                      |                         19 ms |                              — |                        181 ms |
| 34-ply PGN analysis-tree import                                             |                      151.7 µs |                              — | 64.3 µs WASM / 41.2 µs native |
| 100k-puzzle filtered lookup / ladder                                        |                4.13 / 1.34 ms |                              — |                3.41 / 0.83 ms |
| App payload                                                                 |                     21.51 MiB |           historical 28.32 MiB |           28.37 MiB (+31.93%) |
| Complete app logical files, matched runtime/license                         |             344,059,146 bytes |  historical 330,931,412 bytes* |    350,991,382 bytes (+2.01%) |
| Unsigned macOS ZIP size proxy                                               |             137,431,902 bytes |                              — |    141,828,468 bytes (+3.20%) |

The core, desktop, rules, and puzzle rows come from distinct workloads and must not be combined as a single score. The desktop off row is a deliberate behavior difference: Rust exits Stockfish after analysis is turned off, while the TypeScript baseline retains it. The 19 ms baseline re-enable time therefore benefits from an already-running process; Rust's 181 ms includes a restart. *The historical initial-Rust total omitted the matching Chromium license resource and is included only as historical context, not as the matched-runtime final comparison.

## Compared endpoints and machine

The TypeScript reference is full commit `f479f7ce10d7363df7132d616d8ad71b775ec7a7`, extracted at `/private/tmp/kchess-benchmark-ts`. The first Rust migration started at `8f5b7ddcd2b02724d80d8d94c69d33ec61dd3780`. The original Rust endpoint used for the seven-pair comparison is `8f58da7f4d51371001e95d1c33cbd51ed9ad20c2`. The final endpoint is a fixed working-tree snapshot based on that Rust revision, labeled `8f58da7-final-working-tree`; its 38 modified/untracked source files are individually hashed in [the source manifest](raw/final-source-manifest.json), SHA-256 `db262527dbe1b441583a877197ca6a8ceab386fa12e21ca9305969a5e3a36537`. This manifest hash identifies the measured source snapshot; it is not a Git commit SHA.

Measurements were made on macOS 27.0.1 (Darwin 27.0.0), an Apple M5 arm64 machine with 10 logical CPUs and 32 GiB RAM, using Node 24.21.0 and Rust/Cargo 1.94.0 where applicable. These results describe this host and these fixtures. They do not establish Windows or Linux performance, energy use, long-session leak behavior, or notarized-release package size.

The app remains Electron/Nuxt/Vue in both revisions. The same Stockfish 19 lite search engine and settings are used; Rust owns current UCI orchestration, but this study does not compare Stockfish implementations or playing strength. Renderer rules run through the Rust WebAssembly build; core rules run through the Rust native addon. The native path includes the JSON/N-API boundary.

## Core and store workloads

The original core study used seven alternating fresh-process pairs and identical local SQLite fixtures at 1k, 5k, and 50k games. Warm-operation medians pool 12 calls from each process. The initial Rust core had faster module import (22.65→7.80 ms) and empty-profile create/load (4.69→3.90 ms), but the warm history page was 4.5× slower and overview about 12% slower. The 50k insights workload was near parity (1,420→1,399 ms). Whole-worker median max RSS fell from 254 to 144 MiB and peak JS heap from 83 to 8 MiB, but those worker figures include fixture setup, exclude the Stockfish child, and are not desktop memory results.

The final core rerun used three counterbalanced TypeScript/fixed-Rust pairs and matched output signatures in all three. At 50k games, warm insights were 1,418.8 ms TypeScript and 717.5 ms Rust; the direct result confirms the earlier fixed-core improvement. Warm game-page latency was 0.247 ms TypeScript and 0.383 ms Rust, down from the initial Rust result of 1.098 ms after restoring a bounded count cache. The remaining 0.14 ms difference is small for a page request, but Rust is still slower on this endpoint. The overview remained about 1.24 ms slower in Rust (9.75 vs 10.99 ms). Across 1k, 5k, and 50k fixtures, the largest practical gain is the Insights query; ordinary page reads are sub-millisecond in both implementations.

The measured worker max-RSS medians were 260,512 KiB TypeScript and 151,216 KiB Rust (254.4 and 147.7 MiB after converting macOS `maxRSS` from KiB). Peak JS heap was 86,971,304 versus 8,679,960 bytes (82.94 vs 8.28 MiB). Worker CPU time, including fixture seeding, was 24.71 seconds TypeScript and 12.66 seconds Rust. These are whole-worker maxima including fixture setup, not steady-state API memory or desktop RSS; CPU time cannot be attributed to the measured API calls alone.

| Additional core task, 50k games   | TypeScript p50 | Fixed Rust p50 | Context                                                                        |
| --------------------------------- | -------------: | -------------: | ------------------------------------------------------------------------------ |
| 50k-game overview, warm           |       9.752 ms |      10.989 ms | Rust remains about 1.2 ms slower.                                              |
| Study PGN save                    |       0.080 ms |       0.040 ms | Small two-ply write fixture.                                                   |
| Session persistence               |       0.043 ms |       0.033 ms | Small local document fixture.                                                  |
| Stockfish best move, fixed 160 ms |       291.7 ms |       287.8 ms | Seven pairs; same Stockfish 19 lite engine, search moves scheduling-sensitive. |
| Valid 250 ms stop request         |       252.7 ms |       252.0 ms | Seven pairs; stop reached active search in both.                               |

The table below records the remaining warm API medians from the same final three-pair core run. Read and write calls were timed separately after fixture setup; this captures scaling and routine calls without folding seeding into operation latency.

| 50k-game warm read |   TypeScript p50 / p95 |     Rust p50 / p95 |
| ------------------ | ---------------------: | -----------------: |
| Game page          |       0.247 / 0.282 ms |   0.383 / 0.415 ms |
| Library overview   |      9.752 / 10.178 ms | 10.989 / 11.293 ms |
| Insights           | 1,418.75 / 1,433.03 ms | 717.48 / 730.53 ms |

| Fixture rows | Operation           | TypeScript p50 |  Rust p50 |
| -----------: | ------------------- | -------------: | --------: |
|           1k | game page           |       0.237 ms |  0.350 ms |
|           1k | library overview    |       0.238 ms |  0.269 ms |
|           1k | insights            |       70.76 ms |  35.50 ms |
|           1k | library history     |       0.069 ms |  0.076 ms |
|           1k | save settings       |       0.103 ms |  0.213 ms |
|           1k | session persistence |       0.084 ms |  0.043 ms |
|           1k | study PGN save      |       0.149 ms |  0.053 ms |
|           1k | training run save   |       0.087 ms |  0.095 ms |
|           5k | game page           |       0.234 ms |  0.363 ms |
|           5k | library overview    |       0.989 ms |  1.131 ms |
|           5k | insights            |      352.32 ms | 176.64 ms |
|           5k | library history     |       0.067 ms |  0.084 ms |
|           5k | save settings       |       0.078 ms |  0.221 ms |
|           5k | session persistence |       0.054 ms |  0.035 ms |
|           5k | study PGN save      |       0.101 ms |  0.042 ms |
|           5k | training run save   |       0.070 ms |  0.078 ms |
|          50k | game page           |       0.247 ms |  0.383 ms |
|          50k | library overview    |       9.752 ms | 10.989 ms |
|          50k | insights            |    1,418.75 ms | 717.48 ms |
|          50k | library history     |       0.084 ms |  0.103 ms |
|          50k | save settings       |       0.090 ms |  0.236 ms |
|          50k | session persistence |       0.043 ms |  0.033 ms |
|          50k | study PGN save      |       0.080 ms |  0.040 ms |
|          50k | training run save   |       0.064 ms |  0.076 ms |

The original study's 10-second cancellation request exceeded the API's five-second limit and is excluded. A separate valid five-second-maximum cancellation test also confirmed stop behavior. The core `libraryHistory` operation reads the local document library, not synced game history; it starts empty before the small writes. The write workload is not a large-PGN import. The core worker's event-loop maximum includes synchronous fixture seeding and cannot be read as a single user-action stall.

Raw data: [original core study](raw/core-rust-vs-typescript-2026-10.json), [final core comparison](raw/core-final-end-to-end.json), [valid cancellation follow-up](raw/core-engine-cancellation-final.json), [fixed-core comparison](raw/core-after-performance-fixes.json).

The final endpoint includes targeted fixes for the measured hot paths: timezone-preserving Insights grouping and indexed opening aggregation in [insights.rs](/Users/ketansingrole/git/kchess/crates/kchess-core/src/store/insights.rs); a bounded game-page count cache with mutation and database-lifecycle invalidation in [games.rs](/Users/ketansingrole/git/kchess/crates/kchess-core/src/store/games.rs) and [store/mod.rs](/Users/ketansingrole/git/kchess/crates/kchess-core/src/store/mod.rs); and direct UCI-info parsing in [engine/mod.rs](/Users/ketansingrole/git/kchess/crates/kchess-core/src/engine/mod.rs), removing the generic JSON serialize/dispatch/parse round-trip for each info line. The fixes reduced repeated full-tree PGN payload data in [analysisTree.ts](/Users/ketansingrole/git/kchess/crates/kchess-wasm/js/analysisTree.ts), and stream the WASM module when the server returns the right content type in [loadRules.ts](/Users/ketansingrole/git/kchess/apps/desktop/app/utils/loadRules.ts). These source changes explain the implementation shape; only the paired workloads above establish the observed timings.

An exact-timestamp Insights candidate was also checked on a separate 20k-row SQL fixture. It produces identical digests. Its p50 changed from 670.5 to 11.4 ms when 20,000 rows shared only 100 exact timestamps, and from 676.9 to 548.9 ms with 80% unique timestamps. With all timestamps unique, it was slightly slower (671.2 to 674.6 ms). This is a shape-dependent SQL optimization, not a universal win; the all-unique fixture is an important guard against claiming otherwise. Raw data: [timestamp deduplication probe](raw/insights-cache-probe-2026-10.json).

## Renderer rules

The renderer harness used seven alternating fresh-process pairs per candidate, matched inputs, and 200 calls per task after warmup. Output digests matched across TypeScript, Rust WASM, and native Rust. These operations exclude module import and engine initialization and are microbenchmarks, not page-level UI timings.

| Operation                                           | TypeScript | Rust WASM | Rust native |
| --------------------------------------------------- | ---------: | --------: | ----------: |
| Full 34-ply PGN analysis-tree import with variation |   151.7 µs |   64.3 µs |     41.2 µs |
| SAN history                                         |    57.4 µs |   13.7 µs |      7.3 µs |
| Legal destinations                                  |    15.6 µs |   16.1 µs |     11.2 µs |
| Replay 34 plies and return positions                |   104.1 µs |   37.3 µs |     23.8 µs |

The full-tree import is the comparable PGN result. An earlier parser row did unequal work and is informational only; it must not be used to claim a parser regression. Every row is below one millisecond and does not represent full UI rendering or large-document import.

The larger-tree bridge probe finds a tradeoff: importing full 128/384/768-ply trees through WASM takes 0.159/0.442/0.883 ms versus 0.355/1.029/1.830 ms in TypeScript, but serializing those trees to PGN takes 0.088/0.263/0.650 ms in WASM versus 0.021/0.036/0.065 ms in TypeScript. The WASM serialization path is slower and the gap grows with tree size; these remain sub-millisecond in the tested fixtures, but the 768-ply case is a credible optimization target for large studies. The separate navigation and edit bridge probes are in the raw file and are not summarized as a single performance claim.

Raw data: [earlier rules correctness](raw/rules-correctness.json), [earlier rules timings](raw/rules-timing.json), [final rules timings](raw/rules-final-measure.json), [full-tree PGN parity](raw/rules-final-pgn-tree-verify.json), [final full-tree PGN timings](raw/rules-final-pgn-tree-measure.json), [large-tree bridge measurements](raw/bridge-final-measure.json).

## Puzzles and network services

The puzzle service used matched local SQLite fixtures and three worker pairs. On 100k rows, Rust's filtered puzzle lookup was 3.41 ms versus 4.13 ms TypeScript; ladder lookup was 0.83 versus 1.34 ms. First puzzle status after profile reopen was 0.31 versus 16.06 ms, but this includes cold service/database initialization and is not a warm-query comparison. At 10k rows, the same tasks were 2.00 vs 2.86 ms, 0.29 vs 0.82 ms, and 0.37 vs 16.22 ms. Download and install were outside the fixture.

The HTTP harness used deterministic local responses, not the public internet. In the final probe, cold profile/recent-games/TV calls were 6.66/3.70/3.07 ms in TypeScript and 3.13/2.87/2.77 ms in Rust, with all normalized outputs matching, including the corrected TV `gameId` shape. Eight simultaneous identical profile calls made one wire request in each version (102.9 ms Rust, 108.5 ms TypeScript); eight distinct keys still made eight requests in each (411.4 vs 417.9 ms). These loopback numbers reflect client/core overhead plus the fixture delay, not public-service latency.

Raw data: [original puzzle workloads](raw/core-puzzles-2026-10.json), [final puzzle workloads](raw/core-puzzles-final.json), [earlier service probe](raw/network-services.json), [final service probe](raw/network-services-final.json), [earlier profile coalescing probe](raw/network-inflight-after-fixes.json), [final profile coalescing probe](raw/network-inflight-final.json).

## Desktop memory, CPU, and startup

The final desktop comparison used seven paired fresh launches of unsigned macOS arm64 app builds, a fresh disposable profile, and no connected account; visible surface and settings parity passed. Visible Start was 417 ms TypeScript versus 419 ms Rust, Play became enabled at 430 versus 433 ms, and Analysis/History navigation was 84/46 versus 84/45 ms. Those are effectively unchanged. At seven launches, the reported p95 is the sample maximum and should be read as a small-sample range marker:

| Desktop milestone                | TypeScript p50 / p95 | Rust p50 / p95 |
| -------------------------------- | -------------------: | -------------: |
| Visible Start                    |         417 / 443 ms |   419 / 461 ms |
| Play enabled                     |         430 / 461 ms |   433 / 475 ms |
| Analysis navigation              |           84 / 87 ms |     84 / 94 ms |
| History navigation               |           46 / 51 ms |     45 / 47 ms |
| Analysis re-enabled, first depth |           19 / 24 ms |   181 / 187 ms |

While Stockfish was active, whole-app RSS was 1,029.7 MiB for TypeScript and 1,073.8 MiB for Rust (+44.1 MiB, +4.3%); idle Home RSS was 520.7 versus 562.1 MiB (+41.4 MiB, +8.0%). Turning analysis off changed the memory tradeoff: Rust exited Stockfish within one second in 7/7 runs, reducing RSS to 699.3 MiB, while TypeScript retained the engine in 7/7 and remained at 1,042.5 MiB. The Rust off state was 343.2 MiB (32.9%) below the TypeScript retained-engine state, though it remained above the TypeScript idle Home state. After a new move, first depth update took 181 ms Rust versus 19 ms TypeScript, a 162 ms restart penalty from launching the stopped process. This memory benefit depends on using the explicit engine-off behavior; it does not reduce active-search RSS or improve Stockfish's algorithm.

The median process-tree RSS breakdown (MiB) explains the change. Whole-app total is summed per launch; the other component medians are summarized independently and may not add exactly to the total.

| State / process group          | TypeScript |    Rust |
| ------------------------------ | ---------: | ------: |
| Home idle — Browser main       |      217.4 |   232.3 |
| Home idle — renderer           |      158.7 |   184.6 |
| Home idle — GPU                |       96.9 |    96.8 |
| Home idle — engine             |          0 |       0 |
| Home idle — other              |       47.6 |    47.6 |
| Home idle — total              |      520.7 |   562.1 |
| Analysis active — Browser main |      219.0 |   218.2 |
| Analysis active — renderer     |      233.8 |   279.5 |
| Analysis active — GPU          |      141.4 |   141.5 |
| Analysis active — engine       |      387.0 |   387.1 |
| Analysis active — other        |       47.6 |    47.6 |
| Analysis active — total        |    1,029.7 | 1,073.8 |
| 1 s after off — Browser main   |      219.3 |   218.4 |
| 1 s after off — renderer       |      245.7 |   289.2 |
| 1 s after off — GPU            |      143.8 |   143.9 |
| 1 s after off — engine         |      387.0 |       0 |
| 1 s after off — other          |       47.6 |    47.6 |
| 1 s after off — total          |    1,042.5 |   699.3 |

An earlier before/fixed-Rust desktop study likewise found no material change from the low-allocation fixes. In a separate three-pair CPU-quiescence check, active Stockfish used about 3.97 CPU cores in both builds. Median stop latency was 25.7 ms TypeScript and 18.8 ms Rust. After analysis was turned off, Rust exited the engine in all three runs; TypeScript kept roughly 395–396 MiB resident for the ten-second observation, with only about 0.01–0.02 CPU seconds added. This is short-run quiescence evidence, not a long-session leak test. macOS `ps %cpu` is a decaying average and is not used as an instantaneous CPU measure. No energy consumption was measured.

Raw data: [original desktop study](raw/desktop-startup.json), [final packaged comparison](raw/desktop-final-end-to-end.json), [final engine CPU/quiescence check](raw/desktop-cpu-final.json), [earlier engine quiescence](raw/desktop-cpu-quiescence.json), [before/fixed desktop comparison](raw/desktop-after-performance-fixes.json), [renderer memory diagnostics](raw/renderer-memory-gc.json).

## Package footprint

The final normalized comparison used unsigned macOS arm64 directory builds with a matched Chromium license and identical Electron Framework. App payload was 29,751,831 bytes (28.37 MiB) for Rust versus 22,551,559 bytes (21.51 MiB) for TypeScript, +31.93%. Complete logical files were 350,991,382 versus 344,059,146 bytes (+2.01%), and the unsigned ZIP size proxy was 141,828,468 versus 137,431,902 bytes (+3.20%). The ZIPs passed CRC verification, but are not official signed/notarized release artifacts. Rust includes a 13.27 MiB native addon and 3.74 MiB renderer WASM; other dependency payload fell from 11.72 to 1.99 MiB. Electron remains the dominant app footprint. The historical raw full-app sizes omit the matching Chromium license resource, so those absolute totals undercount the rebuilt applications. The normalized TS ASAR exactly matches the original benchmark ASAR hash, supporting that the comparison still uses the same TypeScript app payload. The baseline and current package builders were 26.4.0 and 26.17.0 respectively, so minor non-runtime package-resource differences may remain even though Electron Framework and license files match. The final WASM artifact is 3,920,830 bytes (21 bytes below the earlier default artifact after adding the optional stop parameter; compiler profile is unchanged). The fixed code changes reduced some transfer and serialization work but do not remove the native/WASM requirement.

The workspace already uses fat LTO and one codegen unit. macOS link-time local-symbol removal is configured because post-link `strip` can break dylib loading. A size-oriented WebAssembly profile reduced the WASM file about 15% but made measured rule operations 11–19% slower, so it was rejected and the default optimized profile restored. There is no free, verified whole-app size reduction from changing optimization profiles.

Raw data: [previous canonical size comparison](raw/size.json), [normalized final size comparison](raw/size-final-normalized.json), [final built artifact hashes](raw/final-build-artifacts.json), [rejected size-profile rule timings](raw/rules-opt-s-measure-2026-10.json), [restored default-WASM verification](raw/restored-default-wasm.json).

## Remaining hotspots and verified changes

The results do not show a universal Rust win. Timezone conversion still dominates 50k-game Insights: combining the local weekday/hour conversion reduced the 20k-row slice from about 1,367 to 688 ms, while exact-timestamp dedup helped only duplicate-heavy fixtures and was slightly slower when every timestamp was unique. An hour-bucket alternative was much faster on tested data, but it is not used because equivalence across all timezone and DST cases, including fractional-offset zones, has not been established. The probe preserves local-time and historical-DST output digests.

Large-tree `treeToPgn` remains slower through the WASM JSON boundary, and the final WASM legal-destination task was about 0.5 µs slower than TypeScript. The active and idle desktop process trees remain larger than TypeScript; the prior app-payload comparison also favored TypeScript. The new engine-off behavior changes retained-engine RSS, but requires a process restart before analysis resumes. A single shared engine broker could reduce duplicated retained engine state in a future design, but it is larger work: the current services keep separate engine lifecycles, and any broker must preserve leases, cancellation, search options, and per-analysis context.

The targeted fixes represented in this endpoint are verified by focused regressions and the full checks: timezone-safe Insights grouping and opening-family indexing; bounded count caching with invalidation; profile-request coalescing in [accounts.rs](/Users/ketansingrole/git/kchess/crates/kchess-core/src/lichess/accounts.rs); direct UCI-info parsing; reduced tree-to-PGN request payloads; streaming WASM loading; and the engine idle/off lifecycle in [analysis.rs](/Users/ketansingrole/git/kchess/crates/kchess-core/src/engine/analysis.rs) and the [analysis store](/Users/ketansingrole/git/kchess/apps/desktop/app/stores/analysis.ts). See [the implementation and verification notes](2026-10-10-performance-fixes.md) for measured pre/fix probes and retained regressions. The final validation passed `pnpm run check` (85 suites, 850 tests), production build plus `pnpm run test:e2e` (45 tests), and `pnpm run pack && pnpm run test:packaged` (12 packaged tests). The [source manifest](raw/final-source-manifest.json) identifies the tested source snapshot; [final artifact hashes](raw/final-build-artifacts.json) record the native, WASM, and app binaries.

## Method and limits

The studies use explicit workloads and preserve their own boundary: direct core calls, rule microbenchmarks, loopback network, packaged app launches, and logical package bytes are not combined into one synthetic score. Core and desktop comparisons counterbalanced execution order; rule measurements used seven pairs, and smaller focused follow-ups used three. Fixtures and output signatures were checked before timing. OS file caches are shared between sequential runs. Desktop RSS sums can double-count shared pages.

Do not generalize these local deterministic fixtures to all users or platforms. No benchmark establishes an energy improvement, a long-session memory-leak result, production-network latency, or changed chess strength.
