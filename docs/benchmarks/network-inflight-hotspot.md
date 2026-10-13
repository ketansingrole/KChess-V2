# Concurrent profile request hotspot

The profile cache in the TypeScript baseline coalesces concurrent misses for the same key. The Rust endpoint caches completed values but does not coalesce concurrent misses. The deterministic loopback study reproduced a large same-key difference while its distinct-key control stayed at parity.

Three paired fresh-core runs issued eight simultaneous `CoreApi.profile` calls. The fixture returned deterministic profile JSON after 100 ms. Both sides returned matching normalized results in every pair.

| Burst                        | TypeScript p50 | Rust p50 | TypeScript wire requests | Rust wire requests |
| ---------------------------- | -------------: | -------: | -----------------------: | -----------------: |
| Same username, eight callers |       108.7 ms | 411.9 ms |                        1 |                  8 |
| Eight distinct usernames     |       417.2 ms | 411.8 ms |                        8 |                  8 |

Same-key coalescing cut this fixture's request count from eight to one and p50 latency by 303 ms (about 74%). This is a controlled-loopback result, not a prediction for any particular Lichess connection. The distinct-key control confirms the two versions have nearly identical latency when all eight requests are genuinely independent.

The roughly 412 ms distinct-key burst is consistent with four 100 ms waves through the shared two-request admission limit: the TypeScript baseline's `core/src/services/requestPolicy.ts` admits two active requests, and Rust sets `MAX_ACTIVE = 2` in [`policy.rs`](../../crates/kchess-core/src/lichess/policy.rs). Thus duplicate Rust profile misses contend for the same slots as unrelated traffic. The raw data retains each burst's paths, counts, timing, and normalized results in [network-inflight.json](raw/network-inflight.json); the rerunnable harness is [network-services.mjs](../../tooling/benchmarks/network-services.mjs).

An in-flight coalescing change should preserve the existing cache and request lifecycle contracts:

- Key pending work by the same normalized username and cache kind as the completed profile cache. Bound the pending-key map; do not retain completed futures there or evict a still-running entry in a way that allows duplicate callers to start another request.
- Capture the account epoch before starting the request. After the response, confirm the epoch is unchanged before writing persistent cache data or populating memory cache. Logout, account removal, reset, or `forget_profile` must invalidate older work so it cannot repopulate a cleared key.
- Share the actual request/result for callers of one key, including its stale-cache fallback or error behavior. Do not hold the map lock across network I/O. Preserve the request policy's `profile` usage attribution and let its existing two-slot admission scheduler count one actual wire request.
- Tie shared work to the core lifetime so `close()` cancels the request and drains its entry. If an individual caller can cancel independently, cancelling one waiter must not abort work another waiter still needs; when the final waiter leaves, do not leave an orphan request or an unbounded pending entry.

The measured result shows a confirmed same-key request-fanout regression with exact output parity. It does not yet prove how often the desktop asks for a given profile concurrently in ordinary use; the severity in production depends on those call patterns and actual network latency.
