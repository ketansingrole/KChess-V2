# Performance audit harnesses

These scripts produced the measurements in
[`docs/benchmarks/2026-10-20-performance-audit.md`](../docs/benchmarks/2026-10-20-performance-audit.md).
They use deterministic local fixtures and never contact Lichess or another network service.

## Scripts

| Script                  | Measures                                                                                                                                                                         |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `bench-rules.mjs`       | Domain rules through the native binding, including the JSON boundary: replay, tree import/export, review analysis, opening lookup, material diff, puzzle moves, PGN/study output |
| `bench-core.mjs`        | Core API against a disposable 50k-game SQLite profile: startup, history pages, overview, insights, library                                                                       |
| `bench-renderer.mjs`    | Renderer helpers and rules wrappers: replay memoization, analysis-line formatting, eval/clock/date formatting, game-result payload                                               |
| `bench-network.mjs`     | Eight simultaneous profile reads against a 100 ms loopback fixture; prints elapsed time and actual wire request count                                                            |
| `bench-library.mjs`     | Library snapshot with configurable studies/chapters/archived games, including the document limits                                                                                |
| `chessground-bench.mjs` | Synchronous Chessground `set(config)` versus `setShapes` cost (headless Chromium)                                                                                                |

Every script accepts `--label=<name>` and writes `perf/results/<kind>-<label>.json` (ignored by Git; set `PERF_OUT` to write elsewhere). The rules,
core, renderer and network scripts also accept `--root=<checkout>` so the same benchmark can be
run against the unmodified `main` checkout and the changed worktree.

## Run

```bash
node perf/bench-rules.mjs --root=/path/to/baseline --label=before
node perf/bench-rules.mjs --root=/path/to/worktree --label=after
```

`bench-core.mjs` builds in a folder under the system temporary directory and removes it on exit. It opens the core
once to create the schema, closes it, seeds `games` with `node:sqlite`, then reopens the core for
the timed reads.

`bench-network.mjs` needs `KCHESS_STORE_DEBUG=1` and `KCHESS_TEST_LICHESS_BASE`, which the script
sets itself before importing the core. It only exercises public profile reads, so no credentials
are used.

`bench-library.mjs` saves real studies and archived games through the core API; the seeding time
is reported separately and excluded from the snapshot timings.
