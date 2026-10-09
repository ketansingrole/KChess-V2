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

- Renderer and shared modules cannot import privileged Node/Electron APIs, core or
  main services. Use DesktopApi for privileged work.
- Core (`core/src/services`) cannot import Electron, `apps/desktop/electron/main`, `apps/desktop/electron/preload` or renderer code.
  Host capabilities go through `CorePlatform`; `core/tsconfig.json` typechecks core and
  shared with Node types only, and `pnpm run test:core` builds `core/dist` and drives it
  in plain Node (bundled engine, puzzle worker, library, concurrent isolated Node hosts and CLI).
- Node and CLI hosts use only the core entry and logger; core cannot import either host or
  Vue/Pinia/Nuxt. Game sessions and archive rules in `core/src/domain` have no UI dependency.
- Main imports the core only through `@kchess/core`, `@kchess/core/logger`, `@kchess/core/domain/*`
  and `@kchess/core/contracts/*`; the renderer only through the last two. Desktop bundles the core
  from source via `tooling/core-aliases.ts`; Node frontends use the package's built exports.
- Main, core, shared and preload cannot import renderer code.
- Register IPC through apps/desktop/electron/main/ipc.ts; it authenticates the owned top-level frame
  before validating input and invoking a handler.
- Only puzzleWorker imports puzzle queries. Runtime SQLite imports belong in the
  database owners; store.ts retains its existing legacy-database migration.

A headless capability belongs in `CoreApi`, `CORE_METHODS` and the core service, which
validates its own input; main forwards every core method over IPC. Desktop-only methods
extend `DesktopApi` and register in main. Add every invocation to IPC_CHANNELS,
IPC_CONTRACTS and preload. The shared `apiContracts.ts` tuples validate direct core calls too. Contract tuples require a validator for every argument, including
optional arguments. Declare minimum arity, reject excess arguments and retain
domain/service validation. Startup refuses duplicate or missing handlers.
The OAuth appearance parser deliberately falls back to safe default colors.

## Logging: never swallow errors

- Every `catch` block and every inline `.catch()` must log or rethrow.
  Silent fallbacks (`.catch(() => null)`, empty `catch {}`) fail lint
  (`logging/no-silent-catch`, `logging/no-silent-promise-catch`) and fail
  `tests/unit/logging-guardrail.test.ts`.
- Core and main use `logDebug/logWarn/logError` from `core/src/services/logger.ts` with a
  `[scope]` (file basename, e.g. `engine`, `lichess`): `logDebug` for
  expected/benign (cancel teardown, chain reset, cache miss), `logWarn` for
  recoverable (cache write, reconnect, throttle), `logError` for failures
  needing attention. `logInfo` is for operational milestones (sync completed,
  update available/downloaded). Raw `console.*` in `core/src/services` and `apps/desktop/electron/main` fails
  (`logging/no-raw-console`); only `logger.ts` and `diagnostics.ts` may use it.
- Production context: every failure log carries `method`/`account`/`gameId`/
  `durationMs`/`failures` as applicable, errors go through `errorSummary`
  (preserves `status`/`endpoint`/`code`), long payloads through
  `truncateForLog`, UCI commands through `uciCommandName` (never full
  positions). Never log tokens, passwords, request bodies, or PGNs;
  `redactDiagnostics` covers Bearer/`lip_`/`lio_`/OAuth keys/PKCE/emails plus
  registered secrets. Cancellations (`AbortError`, `SearchCancelled`,
  superseded) log at debug via `isExpectedCancellation`.
- Correlation: `diagnosticSessionId()` tags the startup banner and the
  exported diagnostics JSON so a shared log file splits per launch. IPC
  failures log centrally in `apps/desktop/electron/main/ipc.ts` with method + duration (never
  args); per-IPC timings feed `performanceSnapshot()` in the export.
- Renderer/shared cannot import the main logger. Renderer `console.warn/error`
  is forwarded to the main `DiagnosticLog` file; `console.log/info` is not
  captured, so never use it for errors. Shared uses `console.warn`.
- The only allowed silences carry an explicit `eslint-disable` with
  justification: the diagnostics pipeline itself (logging there would recurse)
  and its fire-and-forget IPC (would loop). Everything else logs with context
  (account/key/gameId/phase) and never logs secrets, tokens, or bodies.

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

tooling/performance-budgets.mjs owns the blocking limits: 12 MiB renderer payload,
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

## Headless host lifecycle

`createKChessCore` owns a platform scope per instance. Different profiles may coexist
in one runtime; duplicate opens of the same profile are rejected. Await `close()` before
reopening that profile;
close cancels network/OAuth, streams, searches and workers, drains requests and reviews,
waits for engine process termination, and then drops profile caches. Closed references
cannot target a subsequent profile. Async callbacks retain their original platform scope.

For worker-isolated clients use `createNodeCore` from the built `core/dist/node.js`.
Each client owns an isolated worker with its own direct core. Await its `close()` in `finally`.
Events return an unsubscribe callback; close detaches subscribers. CLI signals follow the
same shutdown path, with a bounded worker termination backstop.

`ComputerGame`, `LocalGame` and `GameArchive` in `core/src/domain` own game transitions, clocks,
engine turn cancellation and archive identity. `OnlineGame` and `PuzzleSession` also own reusable online and training transitions.
Vue adapters supply reactive state, timer
presentation, sounds and notifications. CLI supplies plain state, terminal I/O and queued
core persistence. Tests must cover the shared controller and the desktop integration.
