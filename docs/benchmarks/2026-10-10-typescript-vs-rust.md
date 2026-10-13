# KChess TypeScript-to-Rust performance study

Core, desktop, five renderer-rule, puzzle, loopback-network, and canonical package-size measurements are complete for the recorded macOS 27.0.1 arm64 host. The TV-channel loopback workload exposed an output-shape mismatch and is excluded from speed interpretation. Results describe endpoint builds and workload boundaries, not a language-only effect.

## What a user would notice

| Surface                      | Measured change                     | Readout                                                                                                     |
| ---------------------------- | ----------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Start / Play enabled         | 415→426 ms / 429→439 ms             | About 10 ms slower to visible and actionable milestones. Analysis/history page navigation differed by 1 ms. |
| Idle app RSS                 | 517.7→559.5 MiB                     | +41.8 MiB (+8.1%); renderer and Browser main account for much of the increase.                              |
| Analysis app RSS             | 1,027.7→1,073.5 MiB                 | +45.8 MiB (+4.5%), including the same Stockfish child.                                                      |
| 100-row game history request | 0.244→1.098 ms                      | +0.85 ms at the core boundary; output rows match.                                                           |
| 50k-game insights            | 1,420→1,399 ms                      | Near parity; both remain about 1.4 seconds on this synthetic fixture.                                       |
| PGN tree import              | 149.7→62.7 µs WASM / 40.3 µs native | Full-tree output parity; the renderer and native paths were about 2.4× and 3.7× faster.                     |
| Installed payload            | 21.51→28.32 MiB                     | +31.7% app payload; complete app logical files rose 2.16%.                                                  |

The overall experience is mixed: most measured page navigation was nearly unchanged, while the larger idle desktop footprint is a clearer regression. Search still uses Stockfish 19; this comparison does not replace the chess engine.

## Scope

This study compares the TypeScript endpoint `f479f7ce10d7363df7132d616d8ad71b775ec7a7` with Rust endpoint `8f58da7f4d51371001e95d1c33cbd51ed9ad20c2`. The baseline source is extracted at `/private/tmp/kchess-benchmark-ts`; the current endpoint is built from the shared checkout. The first Rust migration began at `8f5b7ddcd2b02724d80d8d94c69d33ec61dd3780`.

These endpoints bracket the migration and later integration work. Results describe the measured revisions, not an isolated language swap. The architecture, engine, storage, package, and boundary changes are detailed in [migration context](migration-context.md).

## Measurement plan

Measurements stay separate by boundary so that each result answers a specific question:

| Measurement                        | Boundary                                                                                              | Interpretation                                                                                                                                                                                                                                                                                                           |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Rust core workload benchmark       | Direct core calls and synthetic database fixtures at 1k, 5k, and 50k rows                             | `gamePage` covers the indexed game-history query. `libraryHistory` calls `core.library()` and reads the local-document view, initially empty before the small writes; it is not the synced-game history page. Results are available in [core-rust-vs-typescript-2026-10.json](raw/core-rust-vs-typescript-2026-10.json). |
| Renderer rules microbenchmarks     | TypeScript direct calls, Rust rules through renderer WebAssembly, and Rust rules through native N-API | Compares PGN handling, SAN history, legal destinations, and replay on matching inputs. A separate full analysis-tree PGN import measures equivalent work because the first PGN wrapper measured unequal work. Binding and JSON wrapper costs are included; module import and engine initialization are excluded.         |
| Desktop startup and memory         | Packaged Electron app                                                                                 | Reports visible startup and process-tree memory as end-user observations, not Rust-only costs. Cold/warm state and sampled processes are stated with the results.                                                                                                                                                        |
| Puzzle service                     | Direct public core calls over seeded 10k/100k puzzle databases                                        | Measures filtered puzzle lookup, ladder, and first status after profile reopen. See [core-puzzles-2026-10.json](raw/core-puzzles-2026-10.json).                                                                                                                                                                          |
| Network services                   | Core API calls against loopback HTTP fixtures                                                         | Measures client/cache overhead with deterministic 2 ms responses, not internet latency. TV response-shape parity fails and is excluded. See [network-services.json](raw/network-services.json).                                                                                                                          |
| Package payload and disk footprint | Fresh unsigned app builds for the recorded platform and architecture                                  | Separates logical included file bytes, renderer/main/native/rules/Stockfish payloads, and installed app footprint where the raw data supports those distinctions.                                                                                                                                                        |

Rule workload correctness is checked before timing. The correctness record is in [rules-correctness.json](raw/rules-correctness.json), and timed runs with matching output digests are in [rules-timing.json](raw/rules-timing.json). The full tree-import parity record is in [rules-pgn-tree-correctness.json](raw/rules-pgn-tree-correctness.json), and its timed runs are in [rules-pgn-tree-timing.json](raw/rules-pgn-tree-timing.json). The final size build uses the canonical baseline package without benchmark-only `core/dist` bytes and the same locally installed Electron distribution on both sides. Its raw data is in [size.json](raw/size.json).

## Comparison controls and limits

The core and renderer-rules comparisons use fresh worker processes and matched inputs. The renderer-rules plan uses seven pairs per candidate, alternating which implementation runs first, with batched operations in each process. The desktop comparison uses fresh launches and alternating order. The raw data records host OS and architecture, process/runtime versions, fixture and profile state, cold/warm conditions, iterations, repetitions, and summary statistics. Timing runs were separated to reduce contention.

Stockfish remains the same bundled Stockfish 19 lite WebAssembly engine. Rust owns UCI orchestration in the current endpoint, so engine-task results include that code path but do not measure Rust replacing Stockfish. The renderer still runs Electron/Nuxt/Vue; its chess rules use the WASM binding. Native rule numbers include the N-API JSON bridge. The main SQLite PRAGMAs match, while the SQLite library and call boundary changed.

The engine option policy matches in source: both revisions set `Hash=64` and use a thread budget of one fewer than the available logical CPUs, capped at four; battery mode caps that at two. `bestMove` uses the requested level, and the core workload explicitly uses full-strength `max` with a fixed 160 ms movetime. The desktop Analysis page defaults to engine on, three lines and non-infinite mode in both revisions; its search uses `Hash=128`, `MultiPV=3`, and the shared thread budget. Review uses `Hash=64` and one thread for background work. The engine level ladder and default enabled levels are shared data with matching values. Thread discovery differs (`os.cpus().length` in TypeScript versus Rust `available_parallelism`), and actual thread count also depends on Electron's battery-power callback. Treat raw desktop observations as configuration-matched only if paired runs have the same effective thread count and power state.

Package size reflects dependency removals, the native addon and rules WASM, packaging exclusions, and target-specific artifacts. The renderer remains Electron/Nuxt/Vue. Desktop memory and startup include Electron, renderer, runtime initialization, profile/database state, and OS caching. None of those aggregate measures can be assigned to implementation language alone.

Core workload labels need care: its first “cold” database read happens after `loadData()` has already opened the database, and the OS cache is shared between sequential runs. The history query uses a seeded 50k-game fixture; `core.library()` reads local study/session documents and starts empty. The write workload contains only short two-ply examples, so it is not a large-PGN import benchmark. The core worker's event-loop monitor spans synchronous fixture seeding, and its process RSS includes fixture setup; neither supports a claim about peak UI blocking or core-only memory. That worker does not include a child Stockfish process in its RSS total.

## Results

### Core calls

The core runner used seven alternating fresh-process pairs on macOS 27.0.1 (Darwin 27.0.0), arm64, Apple M5, Node 24.21.0, with 10 logical CPUs and 32 GiB RAM. Rust artifacts used rustc/cargo 1.94.0. Each profile received the same deterministic 5-ply game rows for one account. Warm operation p50 values below pool 12 calls from each of seven processes (`n=84`); cold p50 and startup p50 have seven process samples. Memory reports the median across seven workers.

| Operation                                          | TypeScript p50 |    Rust p50 | Readout                                                                                                                                                                                                                                                          |
| -------------------------------------------------- | -------------: | ----------: | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Module import                                      |       22.65 ms |     7.80 ms | Rust endpoint imported in about 65% less time.                                                                                                                                                                                                                   |
| Empty profile create + `loadData()`                |        4.69 ms |     3.90 ms | Rust endpoint was about 17% lower. Startup parity checked settings keys only.                                                                                                                                                                                    |
| 50k-game `gamePage`, first call after `loadData()` |       1.452 ms |    1.592 ms | Rust endpoint was about 10% slower.                                                                                                                                                                                                                              |
| 50k-game `gamePage`, warm                          |       0.244 ms |    1.098 ms | Rust endpoint took about 4.5× as long; this is the clearest core-call regression in the measured reads. The page returns 100 rows through the N-API JSON bridge, so payload conversion may contribute; the runner does not separate SQL time from serialization. |
| 50k-game `gameLibraryOverview`, warm               |       9.765 ms |   10.903 ms | Rust endpoint was about 12% slower.                                                                                                                                                                                                                              |
| 50k-game `insights`, warm                          |    1,420.46 ms | 1,398.67 ms | Rust endpoint was about 1.5% lower; both take about 1.4 seconds on this synthetic fixture.                                                                                                                                                                       |
| `saveSettings`, warm                               |       0.088 ms |    0.232 ms | Rust endpoint was about 2.6× as long, with both medians below 1 ms.                                                                                                                                                                                              |
| `studySavePgn`, warm                               |       0.086 ms |    0.040 ms | Rust endpoint was about 53% lower for the small two-ply write fixture.                                                                                                                                                                                           |
| `sessionPersistence`, warm                         |       0.045 ms |    0.033 ms | Rust endpoint was about 27% lower for the small fixture.                                                                                                                                                                                                         |
| Full-strength Stockfish `bestMove`, fixed 160 ms   |      295.09 ms |   290.59 ms | Similar end-to-end latency with the same Stockfish asset and settings; this does not compare chess engines.                                                                                                                                                      |

Across the whole worker, median `maxRSS` was 254.2 MiB for TypeScript and 144.0 MiB for Rust; median peak JS heap was 83.0 MiB and 8.2 MiB. These values include imports, fixture seeding, all measured calls and writes, and do not include the child Stockfish process. They are whole-worker resource results, not core-only or desktop-app memory.

The whole-worker maximum event-loop gap was 18.6 s for TypeScript and 244 ms for Rust; event-loop delay p95 was 948 ms and 12 ms. Fixture seeding is synchronous, and the sampling window includes fixture work and an awaited workload burst. These readings do not establish the time a single UI action blocks or predict desktop responsiveness.

After normalizing only the generated study UUID reference between isolated profiles, all seven core pairs pass output parity across the checked signatures. An initial 10-second cancellation request exceeded the API's 5-second limit and is excluded. A separate valid 5-second maximum search cancellation follow-up requested stop at 250 ms in all seven pairs; searches rejected as superseded after a median 251.5 ms (TypeScript) and 252.9 ms (Rust), including timer dispatch and stop propagation. The empty-profile check compared settings keys, not full default values.

The core results do not support a single overall speedup claim: the large-history page call regressed despite faster import/startup and slightly lower large-fixture insights latency.

### Packaged desktop startup and memory

The desktop benchmark launched fresh unsigned macOS 27.0.1 arm64 app builds in seven paired runs, with disposable profiles and no connected account. Raw results are in [desktop-startup.json](raw/desktop-startup.json). Navigation targets and engine update behavior passed visible parity checks. All runs reported battery power off. Startup is measured from just before Electron launch until the visible Start section or enabled Play the computer action; it is not process initialization alone.

| Observation                                   | TypeScript p50 |    Rust p50 |            Change |
| --------------------------------------------- | -------------: | ----------: | ----------------: |
| Visible Start                                 |         415 ms |      426 ms |             +2.7% |
| Play the computer enabled                     |         429 ms |      439 ms |             +2.3% |
| Analysis page navigation                      |          86 ms |       87 ms |             +1 ms |
| History page navigation                       |          50 ms |       51 ms |             +1 ms |
| First Stockfish analysis update               |         240 ms |      235 ms |             −5 ms |
| Home idle whole-app process-tree RSS          |      517.7 MiB |   559.5 MiB | +8.1% (+41.8 MiB) |
| Analysis with engine active, process-tree RSS |    1,027.7 MiB | 1,073.5 MiB | +4.5% (+45.8 MiB) |

The process-tree totals sum Electron main, renderer, GPU, utility and engine processes. On macOS, RSS sums may double-count shared pages. Electron's own process metrics show idle Browser+Renderer+GPU+Utility working-set RSS of about 517.6 MiB vs 559.4 MiB, so the idle increase is already present without a running search. In the process breakdown, idle renderer RSS rose about 26 MiB and Browser main RSS about 15 MiB; with analysis active, renderer RSS was about 47 MiB higher. This accompanies the added renderer WASM and other endpoint changes, but does not isolate their individual memory cost. With analysis active, the Stockfish child adds about 386 MiB in both versions. It remains resident after the engine is switched off, so the post-analysis total still includes it. A separate three-pair cancellation check measured engine cumulative CPU increasing by about 16.4 seconds over roughly four wall-clock seconds while analysis was active (about four cores). After stopping, CPU advanced only about 0.01–0.02 seconds over nine seconds, while roughly 386 MiB remained allocated. This confirms the search became CPU-idle during the short observation, not that long-session memory is leak-free; see [desktop-cpu-quiescence.json](raw/desktop-cpu-quiescence.json). The earlier Rust core-worker RSS improvement therefore does not translate into lower measured whole-app desktop RSS. Visible startup was about 10 ms later and the Play action about 10 ms later; page navigation differed by 1 ms. These small user-task differences are close, while the desktop RSS increase is the clearest measured resource regression.

Electron launch resolution itself was faster in the Rust package (298 vs 238 ms median), while visible and actionable UI milestones were slightly slower. This difference shows why launch-call completion alone is not a user-visible startup measure. The desktop startup raw retains `psReportedCpuPercent`, which macOS defines as a decaying average over up to one minute, so it is not treated as a point-in-time CPU measure. The separate cumulative-CPU quiescence check found about 3.96 CPU-seconds per wall-second for both variants while Stockfish was searching. In the 9.5-second post-stop window, median CPU use fell to about 0.21% of one core for TypeScript and 0.11% for Rust across only three paired runs; treat both as near-zero for this sample, not as a version advantage.

### Renderer rules microbenchmarks

The rules harness used seven alternating fresh-process pairs per candidate on macOS 27.0.1 arm64 with Node 24.21.0. Raw timings are in [rules-timing.json](raw/rules-timing.json). It ran 200 iterations per task after five warmups and excluded module import and binding initialization. Inputs were matched across the TypeScript implementation, Rust renderer WASM and Rust native N-API path. All four operation digests matched in every pair. Reported values are the median of seven per-process per-operation times, normalized to microseconds per operation.

| Operation                                                        | TypeScript | Rust renderer WASM | Rust native N-API |
| ---------------------------------------------------------------- | ---------: | -----------------: | ----------------: |
| Parse first PGN mainline game (unequal work; informational only) |   10.94 µs |           72.02 µs |          47.05 µs |
| Import full 34-ply PGN analysis tree including variation         |  149.72 µs |           62.73 µs |          40.29 µs |
| Build SAN history                                                |   57.47 µs |           13.77 µs |           7.26 µs |
| Legal destinations                                               |   14.96 µs |           15.91 µs |          11.23 µs |
| Replay 34 plies and return positions                             |  103.84 µs |           37.37 µs |          23.79 µs |

On the equivalent full-tree PGN import, Rust was about 2.4× faster through renderer WASM and 3.7× faster through native N-API. SAN-history generation was about 4.2× faster in WASM and 7.9× faster in native; replay was about 2.8× and 4.4× faster. Legal destinations were near parity in WASM (about 6% slower) and about 25% faster in native. The first PGN row is not an equivalent-work parser comparison: the TypeScript side calls `chessops.parsePgn` and projects headers plus mainline SAN, while `pgnMainline` parses then validates and replays the main line, computing UCI, FEN, mover and check for each move. The digest projects only headers and SAN, so it does not mean both implementations performed the same work. Do not read the PGN row as evidence of a Rust parser regression or language effect. The separate full-tree import row covers equivalent parse-and-validate work. Each value is below a millisecond, and the synthetic single-game fixture does not represent large PGN import, full page rendering or end-to-end UI latency. Native timings include the JSON/N-API bridge; renderer timings include the WASM call wrapper.

### Puzzle database workloads

The puzzle benchmark used three paired worker-process runs with matched 10k- and 100k-row SQLite fixtures. It seeded the Rust service schema directly, validated returned counts, rating/theme filters and legal solution lines, then measured 20 calls per API and fixture size in each worker. Warm queries below pool 60 samples (`n=60`); `status` is a first call after reopening each fixture profile (`n=3`) and includes puzzle-service/database initialization. Raw measurements are in [core-puzzles-2026-10.json](raw/core-puzzles-2026-10.json).

| 100k puzzle fixture                         | TypeScript p50 | Rust p50 | Change |
| ------------------------------------------- | -------------: | -------: | -----: |
| `localPuzzles` filtered query               |       3.967 ms | 3.269 ms | −17.6% |
| `localLadder`                               |       1.368 ms | 0.804 ms | −41.2% |
| First `puzzleDbStatus` after profile reopen |      15.047 ms | 0.309 ms | −97.9% |

The first `puzzleDbStatus` includes cold service/database initialization, so it is not a warmed status-query comparison. These fixtures bypass puzzle download and installation.

### Loopback network-service workloads

The network runner used three alternating process pairs and a deterministic local HTTP fixture with a 2 ms response delay. It exercises each revision's `CoreApi`, client parsing and caching, not internet latency or service rate limits. In [network-services.json](raw/network-services.json), `profile` output matched and warm calls made no HTTP request. Cold profile latency was 6.794 ms (TypeScript) vs 3.166 ms (Rust); warm cached latency was 0.033 vs 0.046 ms (`n=12`). `recentGames` output also matched; cold latency was 3.707 vs 2.887 ms and warm was 3.052 vs 2.766 ms (`n=12`), with one fixture HTTP request on every warm call. The response delay dominates much of those recent-games numbers.

TV-channel output did not match, so its timing is excluded. The deterministic fixture supplied `gameId`; TypeScript's `TvChannel` contract uses `gameId`, while the Rust `TvChannel` struct serializes its `game_id` field in snake_case. This is a concrete compatibility difference in the measured Rust endpoint, not a performance result.

### Packaged size: unsigned macOS 27.0.1 arm64 build

The final size run used fresh unsigned `electron-builder` directory builds with the same locally installed Electron distribution. The Rust release artifact used `lto = "fat"` and `codegen-units = 1` from the workspace release profile (the compiler default optimization level is 3). Logical file bytes exclude symlinks, APFS allocation and compression; the ASAR count includes unpacked entries once. `appPayloadBytes` excludes the voice archive according to each revision's packaging rules. The ZIP is an unsigned `ditto` archive of the `.app`, so it is a download-size proxy rather than an official signed/notarized release ZIP or DMG.

| Measure                   |                 TypeScript |                       Rust | Change |
| ------------------------- | -------------------------: | -------------------------: | -----: |
| Included app payload      |   22,551,559 B (21.51 MiB) |   29,693,058 B (28.32 MiB) | +31.7% |
| Full `.app` logical files |              323,946,841 B |              330,931,412 B | +2.16% |
| Unsigned ZIP proxy        | 135,310,796 B (129.04 MiB) | 139,771,455 B (133.30 MiB) | +3.30% |

The payload composition shifted substantially: Rust build includes a 13,859,128-byte native addon and 3,920,851-byte renderer rules WASM; TypeScript has neither. The renderer payload rose from 8,104,132 to 11,948,689 bytes, while the `otherDependencies` category fell from 12,292,168 to 2,086,925 bytes. The latter is a packaging category, not a pure JavaScript measurement. Stockfish stayed exactly 1,712,228 bytes in both builds. These totals show that dependency pruning offset much of the Rust artifacts' size in the complete app, but the shipped app payload itself is about 7.1 MB larger.

## Recommendations

Fix the TV-channel serialization to preserve the existing `gameId` contract and add a JSON contract regression before shipping. Profile idle and analysis renderer memory next: the process breakdown shows roughly +26 MiB at idle and +47 MiB during active analysis, even though the headless Rust worker used less memory. If query latency is a priority, split the 50k history call into SQL and JSON bridge costs; its measured increase is 0.85 ms, so it is a lower priority than the desktop memory increase and the 1.4-second insights workload, which remained slow in both revisions. The shared Stockfish result was near parity, so the current evidence does not call for engine-search tuning.

## Confidence and gaps

All measurements came from one Apple M5 arm64 machine running macOS 27.0.1. There is no Windows or Linux runtime, memory, or package-size comparison, and no energy measurement. App builds were unsigned; the ditto ZIP is a proxy, not a signed/notarized release ZIP or DMG. The benchmark did not measure long-session memory growth, large real-world PGN imports, very large studies, or real Lichess service latency. The full PGN fixture is one 34-ply game with a two-ply variation. The desktop cancellation check only observes 9.5 seconds after stop and does not establish whether resident Stockfish memory is a leak. The Rust endpoint includes work after the initial port, dependency and packaging changes, and a different storage/runtime boundary, so all endpoint-level differences remain confounded.

## Reproduction

The pinned source endpoints are listed above. The baseline snapshot is `/private/tmp/kchess-benchmark-ts`; the Rust checkout is `8f58da7f4d51371001e95d1c33cbd51ed9ad20c2`. The measurements used Node 24.21.0, pnpm 11.19.0, rustc/cargo 1.94.0, and the locally installed Electron distribution. In each checkout, install from its lockfile and run `pnpm run build`; the Rust build generates the host native addon. `pnpm run build` does not produce the historical headless core bundle, so the TypeScript baseline also needs `node node_modules/vite/bin/vite.js build -c core/vite.config.ts --logLevel warn`. For canonical app-size packaging, temporarily move that benchmark-only `core/dist` out of the TypeScript baseline app root, package both endpoints against the same locally installed Electron, then restore `core/dist` before direct core measurements. The desktop-size runner wrote `/private/tmp/kchess-typescript-benchmark.zip` and `/private/tmp/kchess-rust-benchmark.zip`; verify them with `unzip -t`.

```sh
# Direct core and fixture workloads
node tooling/core-benchmark.mjs --pairs=7 --sizes=1000,5000,50000
node tooling/core-engine-cancellation.mjs
node tooling/core-puzzle-benchmark.mjs
node tooling/benchmarks/network-services.mjs

# Rule correctness and paired timing
node tooling/benchmarks/rules-benchmark.mjs --verify docs/benchmarks/raw/rules-correctness.json
node tooling/benchmarks/rules-benchmark.mjs --measure docs/benchmarks/raw/rules-timing.json
node tooling/benchmarks/rules-benchmark.mjs --verify-pgn-tree docs/benchmarks/raw/rules-pgn-tree-correctness.json
node tooling/benchmarks/rules-benchmark.mjs --measure-pgn-tree docs/benchmarks/raw/rules-pgn-tree-timing.json

# Packaged desktop and artifact measurements
node tooling/with-branded-electron.mjs node tooling/benchmarks/desktop-startup.mjs "$PWD" /private/tmp/kchess-benchmark-ts --runs=7 --packaged
KCHESS_PACKAGED_EXEC_PATH="$PWD/dist/mac-arm64/KChess.app/Contents/MacOS/KChess" node tooling/with-branded-electron.mjs node tooling/benchmarks/desktop-cpu-quiescence.mjs
node tooling/benchmarks/size.mjs /private/tmp/kchess-benchmark-ts "$PWD" --zip-dir=/private/tmp
```

Run timing only while the host is quiet. Preserve the raw JSON because it records each pair, output parity, fixture conditions, sample counts, and machine metadata.

## Validation

The completed checkout passed `pnpm run check:fast` (formatting, lint, native build, type checks, and 845 tests across 82 suites). All nine benchmark scripts passed `node --check`; the four renderer-rule operations and the full-tree PGN task were rerun for output parity across TypeScript, WASM and native implementations. Both production builds, canonical unsigned package validation, and ZIP CRC checks passed. The first sandboxed `check:fast` attempt could not open a local HTTP fixture; it passed on rerun outside that restricted sandbox, with no product-code change.
