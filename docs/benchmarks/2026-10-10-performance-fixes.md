# Performance fixes and verification

This records the first round of fixes. The [final end-to-end comparison](2026-10-10-end-to-end-performance.md) includes the subsequent engine-off memory release, retirement-timer correction, exact-timestamp reuse and fresh final measurements. Statements below about an unchanged engine retention policy apply to this earlier round.

This follows the [root-cause study](2026-10-10-performance-root-causes.md). The original reports/raw files describe the endpoints before these fixes and remain preserved. The implementation is currently an uncommitted working-tree change based on `8f58da7`; no release, tag or push was made.

## Implemented changes

- Insights materializes one combined local weekday/hour conversion per game, retaining SQLite's local-time semantics. No unsafe UTC-hour bucketing was introduced. Opening-family aggregation uses a hash index alongside its ordered vector, preserving first-seen order and ties.
- Concurrent cold profile/rating-cache requests share one pending fetch per normalized key and account epoch. The pending map is bounded; failed/cancelled leaders clean up and followers can take over. Logout, forget/reset and core shutdown retain their invalidation/cancellation behavior. Profile-generation lookups do not create an entry for each public username.
- History page totals have a bounded 128-entry cache keyed by account/result/rated filters. Pagination does not affect the count key. Game imports, upserts, pruning, clear/remove/logout, resets, debug SQL and database close invalidate it under the database lifecycle lock.
- Tree→PGN serialization sends only the metadata the Rust writer reads: root FEN, ordered child SAN/annotations and headers. Child position FEN/UCI/ply snapshots stay in the renderer. Variations, comments, starting comments, NAGs and custom headers remain preserved. The existing persistence debounce and reactive node identities remain intact.
- WASM loading uses streaming instantiation for `application/wasm` responses, avoiding the explicit source ArrayBuffer. Hosts with other content types retain buffered loading. HTTP and compilation errors still propagate. The rules still initialize before any rule-dependent UI.
- TV channels now serialize `gameId`, restoring the shared contract's camelCase shape.

## Memory and size scope

Streaming removes an avoidable source-buffer copy; it does not remove the required native addon, WASM linear memory or compiled code. The prior GC study found no additional large retained JavaScript heap in the sampled phases. No forced GC, smaller unverified WASM stack, feature removal or binary stripping was added. Stockfish search implementation/settings and the existing idle-engine policy remain unchanged. Thus these fixes do not claim to eliminate the entire whole-app RSS/package-size difference caused by the migration.

## Regressions retained

Focused regressions cover SQL calendar parity near historical boundaries, opening ordering across 1,024 families, count-cache bounds/filter reuse/upserts/deletions/imports/debug SQL/reopen, shared versus distinct profile fetches, cancelled leaders, invalidation during pending work, TV wire shape, PGN annotation/header preservation and streaming/buffered loading failures.

The count-cache verification initially exposed a fixture SQL lazy-open regression; it was corrected and retained as a first-call debug-execution regression. The Rust debug test runs in an isolated child process rather than mutating environment variables in a parallel test runner.

## Measured results with the original WASM compiler profile

| Workload                                                       |               Pre-fix Rust |           Fixed Rust | Result                                                      |
| -------------------------------------------------------------- | -------------------------: | -------------------: | ----------------------------------------------------------- |
| 50k-game insights, warm full API                               |                 1,398.7 ms |             714.1 ms | 48.9% lower latency                                         |
| 50k-game history page, warm API                                |                   1.098 ms |             0.379 ms | 65.5% lower latency; cold counts still cost a query         |
| Eight simultaneous identical profile calls                     | 411.9 ms / 8 wire requests | 103.0 ms / 1 request | 75.0% lower latency, 87.5% fewer wire requests              |
| 20k distinct opening labels, full API with explicit Tokyo zone |                 1,246.2 ms |             510.8 ms | Combined SQL/grouping fixes remove 59.0% of this latency    |
| 768-ply tree→PGN WASM helper                                   |                   0.852 ms |             0.649 ms | 23.8% faster; request 92,830 → 20,934 bytes (77.4% smaller) |
| 768-ply tree→PGN native helper                                 |                   0.772 ms |             0.597 ms | 22.7% faster with the same compact request                  |

The core run used three counterbalanced fresh-process TS/fixed-Rust pairs, 50k identical games and 12 warm calls per worker. All three pairs matched full output signatures. The unchanged TS control remained near 1.41 seconds for insights. The pre-fix Rust core figures come from the preserved original seven-pair study, not concurrent before/fixed worker pairs; modest differences should not be interpreted beyond that design. Raw: [core-after-performance-fixes.json](raw/core-after-performance-fixes.json).

The opening stress probe was one fresh process per endpoint with three warm high-cardinality samples and seven ordinary samples, using explicit `TZ=Asia/Tokyo`. Fixed Rust ordinary insights was 508.9 ms and the 20k-label case 510.8 ms: the former ~263 ms cardinality penalty is absent in this fixture. The historical TS stress control was 1,009.0 ms in the same new probe. Raw: [insights-hotspots-after-fixes.json](raw/insights-hotspots-after-fixes.json). This is a synthetic stress case, not typical opening cardinality.

The network probe used three fresh-process pairs with a deterministic 100 ms fixture and exact normalized-output parity. Distinct-key controls stayed near 413 ms Rust / 422 ms TS with eight genuinely independent requests. Raw: [network-inflight-after-fixes.json](raw/network-inflight-after-fixes.json).

The bridge probe used seven paired fresh processes per candidate and batches of 20 operations at 128/384/768 plies. Output hashes matched the historical TS implementation. It includes projection cost; this is not merely a smaller-byte calculation. Import, navigation and existing-child reuse stayed in their prior ranges. Raw: [bridge-hotspots-after-fixes.json](raw/bridge-hotspots-after-fixes.json).

## Direct packaged before/fixed comparison

Seven counterbalanced fresh-profile pairs compared the preserved pre-fix Rust package against the newly built fixed package, both unsigned macOS arm64 and both using the original WASM compiler profile. All 14 launches succeeded and visible surface labels matched.

| Metric, median per candidate   |      Before |       Fixed |
| ------------------------------ | ----------: | ----------: |
| Actionable Play button         |      428 ms |      432 ms |
| Analysis navigation            |       85 ms |       85 ms |
| History navigation             |       51 ms |       50 ms |
| Whole-tree idle RSS            |   562.1 MiB |   560.8 MiB |
| Whole-tree active-analysis RSS | 1,073.4 MiB | 1,072.8 MiB |
| Idle renderer RSS              |   185.0 MiB |   184.5 MiB |
| Active renderer RSS            |   279.7 MiB |   279.9 MiB |

These are effectively unchanged desktop startup/navigation and memory results. The copy-reduction fixes do not demonstrate a material whole-app RSS win; do not label the ~1 MiB difference as one. RSS sums can double-count shared pages and the active engine retains the same settings. The [packaged protocol probe](raw/wasm-protocol-after-fixes.json) returned `application/wasm` with status 200, confirming the new streaming branch is eligible in the actual packaged app. Raw: [desktop-after-performance-fixes.json](raw/desktop-after-performance-fixes.json).

## Verification of the fixes above

`pnpm run check` passed formatting, lint, notices, types, Rust tests/coverage, native build, contract generation, smoke/core integration, all 84 unit suites / 849 tests and structural performance budgets. Rust coverage remained above all enforced floors (lines 64.39%, regions 60.04%, functions 61.29%). `pnpm run build && pnpm run test:e2e` passed all 45 production UI tests. `pnpm run pack && pnpm run test:packaged` produced a verified unsigned arm64 app and passed all 12 packaged tests. No isolated benchmark app, engine or worker remained after these runs.

## Rejected compiler-size experiment

An isolated `CARGO_PROFILE_WASM_OPT_LEVEL=s` build reduced the WASM file from 3,920,851 to 3,336,915 bytes (14.9%). It passed all 25 native-rule tests with `KCHESS_FUZZ_SCALE=20` and explicit 1,024-ply import/tree/PGN byte parity against native Rust. However, direct measurements using the same current wrappers and separate original/candidate WASM inputs showed consistent regressions:

| Rule operation          | Original opt-level 3 | Candidate opt-level s | Regression |
| ----------------------- | -------------------: | --------------------: | ---------: |
| PGN mainline processing |            0.0715 ms |             0.0850 ms |      18.8% |
| SAN history             |           0.01367 ms |            0.01520 ms |      11.2% |
| Legal destinations      |           0.01638 ms |            0.01885 ms |      15.1% |
| Replay positions        |           0.03741 ms |            0.04191 ms |      12.0% |

Tree operations also regressed by roughly 11–23%. The smaller compiler profile was rejected; `Cargo.toml` remains unchanged and the default profile was restored with the official build tool. No desktop memory saving is claimed for this rejected candidate. This prevents a file-size optimization from quietly undoing rule-performance gains.

Raw: [original-profile rules](raw/rules-opt-3-s-wrapper-measure-2026-10.json), [size-profile rules](raw/rules-opt-s-measure-2026-10.json), [original-profile trees](raw/bridge-opt-3-s-wrapper-measure-2026-10.json), [size-profile trees](raw/bridge-opt-s-measure-2026-10.json), [1,024-ply parity](raw/wasm-depth-1024-2026-10.json).

The restored default WASM is byte-for-byte identical to the artifact in the already-verified production build: 3,920,851 bytes, SHA-256 `0b3fe84d139f93f13e6e32b4ab8acc23a22aa3875ee546b2d88d981fd5239eb5` ([restoration check](raw/restored-default-wasm.json)). All 13 benchmark scripts passed syntax checks and final tooling lint/format checks. Product source and compiler configuration match the full-check/e2e/packaged-tested state.
