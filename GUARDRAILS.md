# Guardrails and verification

Run `pnpm run check:fast` while editing. It checks tracked changes against HEAD,
staged changes and new files. Use `pnpm run check:fast --base origin/main` to
include committed branch changes. Formatting and lint run on changed files;
unit tests run directly for changed tests and through Vitest's dependency graph
for changed source. Shared contracts, configuration, scripts, dependencies and
deleted files broaden verification to all unit tests and types. This is an editing
aid; related tests do not establish complete regression coverage.

Before merging, run `pnpm run check`. It checks formatting, lint, renderer/main/test
types, smoke tests, all unit tests and structural performance budgets. Production
verification remains `pnpm run build && pnpm run test:e2e`. Packaging changes also
require `pnpm run pack && pnpm run test:packaged`. CI runs desktop and packaged tests
on macOS, Windows and Linux. All four CI jobs must pass on the current merge base.

## Ownership enforced by lint

- Renderer and shared modules cannot import privileged Node/Electron APIs or main
  services. Use DesktopApi for privileged work.
- Main, shared and preload cannot import renderer code.
- Register IPC through src/main/ipc.ts; it authenticates the owned top-level frame
  before validating input and invoking a handler.
- Only puzzleWorker imports puzzle queries. Runtime SQLite imports belong in the
  database owners; store.ts retains its existing legacy-database migration.

Add every invocation to DesktopApi, IPC_CHANNELS, IPC_CONTRACTS, preload and main
registration. Contract tuples require a validator for every argument, including
optional arguments. Declare minimum arity, reject excess arguments and retain
domain/service validation. Startup refuses duplicate or missing handlers.
The OAuth appearance parser deliberately falls back to safe default colors.

## Lifecycle ownership

Use RequestScope for replaceable work: next() aborts the previous signal; capture()
tracks the current generation. Pass the signal to cancellable local I/O and check
current() after every await before committing state. Across IPC, explicitly stop
the main service as Watch does; rejecting a late reply alone does not stop work.

Use SubscriptionScope for one listener set per owner and detach it on teardown.
Use withEngineLease for engine configuration/search; it releases scheduler
ownership in finally on success, failure and cancellation. UciController still
owns process deadlines and UCI pipes.

For each escaped regression, preserve the reproducer and test its sequence:
late replies, overlapping requests, cancellation, lost credentials, interrupted
writes and recovery. Generated tests cover Watch acknowledgment ordering and
live-game recovery, alongside existing PGN preservation, study conflict, worker
rollback and engine lifecycle tests. Fast-check prints replay seeds and paths on
failure; keep a concrete reproducer when a generated case finds a defect.

## Budgets and measurements

scripts/performance-budgets.mjs owns the blocking limits: 12 MiB renderer payload,
32 MiB app payload, 16 MiB individual files, and a 100-row library fixture page
under 64 KiB. The fixture measures representative short games; it is not a universal
limit on user games. Build/package verification checks actual payloads.
GIF tests require at most one unacknowledged transferred frame, detached producer
buffers across 600 frames, cancellation and a single worker termination.
Study tests require navigation to reuse PGN and repeated edits to serialize once.

`pnpm run check` and `check:fast` record step durations and failures under
test-results/guardrails. `pnpm run check:report` creates a JSON summary; CI also
publishes timings and desktop flake rates to its job summary and retains reports
for 30 days. Desktop JSON reports distinguish recovered retries from clean passes.
Timing trends are informational; structural limits fail verification.

Review feedback latency, flaky/attempted desktop tests and released regressions
monthly. Bug reports include last working version and trigger sequence. Apply the
regression issue label only to confirmed bugs that previously worked in a released
version; unknown reports stay unclassified. CI reports open and closed regression
issues created in the last 30 days using read-only GitHub access. Missing access or
classification is reported as unavailable. The count depends on triage, rather
than treating every issue labeled bug as a regression. Use the previous month's reports to
compare the same runner and fixtures before setting additional timing gates.
