# KChess workspace

```text
core/                    Headless chess library
  src/domain/            Pure chess, game, training and document rules
  src/contracts/         Core API, events, types and argument validation
  src/services/          Engines, networking, credentials and persistence
  tests/unit/
crates/                  Rust core, built with Cargo (`Cargo.toml` workspace)
  kchess-domain/         Chess, PGN and library document rules with chessops semantics
  kchess-node/           `@kchess/native`: the N-API module the core loads
hosts/node/              Reusable isolated Node host and OS capabilities
apps/desktop/            Electron + Nuxt application
  electron/main/         Windows, native integration and authenticated IPC
  electron/preload/      Sandboxed renderer bridge
  electron/renderer/     Electron build entry; the UI lives in app/
  app/                   Nuxt views, components and reactive stores
  contracts/             Desktop API, IPC and renderer diagnostics
  public/                Static renderer assets
  build/                 Icons and packaging resources
  tests/unit/
  tests/e2e/
apps/cli/                Terminal application
  src/
tooling/                 Repository checks, fixtures, asset and release tooling
tests/                   Shared fixtures and repository guardrail regressions
```

Each library and application has its own package manifest and build configuration.
The root package coordinates development, verification and releases. A single pnpm
lockfile covers every workspace package. Root development dependencies also support
the repository's integration tests; shipped dependencies are declared by their owners.

Core never imports a host or application. Domain and contract modules remain pure and
cannot import Node, Electron or frontend frameworks. Frontends may share these pure
modules; privileged services are available through the core entry and logger only.
Electron implements `CorePlatform`; the reusable Node host supplies worker isolation,
OS credentials and browser login. CLI owns terminal input and presentation.

The core is moving to Rust one CPU-bound service at a time. `core/src/services/rules.ts`
routes replay, PGN validation, library document decoding, review analysis, Lichess line
conversion, study PGN export and the puzzle database sampler to `@kchess/native`, which the
core requires (there is no TypeScript fallback). Rules the renderer still needs synchronously
(`treeFromPgn`, `replaySetup`, `replay`, `analyseReview`, `studyDocumentPgn` and the chess
rules) keep a TypeScript twin in `core/src/domain`; the seeded differential tests in
`core/tests/unit/native-rules.test.ts` keep both identical (`KCHESS_FUZZ_SCALE=20` searches
deeper). Rules that exist only in Rust are pinned to `core/tests/unit/native-golden.json`,
the outputs of the TypeScript rules they replaced. Contracts check the shape of a saved game
or study command; the core service replays and validates it in Rust. The one deliberate
difference: an en passant square that a piece occupies (reachable only from a typed FEN) is
dropped rather than kept. Review figures go through `exp`, which the Rust rules compute as V8
does (fdlibm, with the multiply-adds V8's compiler fuses on arm64), so results match to the
bit; a test compares it with `Math.exp` on every platform.

Desktop IPC contracts extend the core validation tuples. Core validates direct calls
without loading desktop contracts, and desktop replaces the settings tuple with its
complete frontend settings shape. Import rules enforce these boundaries.

Run the existing root commands:

- `pnpm run dev`: one branded Electron + Nuxt development session.
- `pnpm run cli -- --help`: build the headless packages and run CLI.
- `pnpm run build:core`: build core, Node host and CLI into their own `dist/` folders.
- `pnpm --filter @kchess/core build`: build core alone, including declarations.
- `pnpm --filter @kchess/node build`: build the reusable Node host and declarations.
- `pnpm run build:native`: build the Rust rules for this host (also part of `build` and `dev`).
- `pnpm run check:rust`: rustfmt, clippy and Rust unit tests.
- `pnpm run benchmark:core`: TypeScript vs Rust rules on seeded fixtures, in separate processes.
- `pnpm run build`: typecheck and build desktop into `apps/desktop/out/` and `.output/`.
- `pnpm run check:fast`: checks for editing; `pnpm run check`: full repository checks.
- `pnpm run test:e2e`: production desktop regressions with isolated profiles.
- `pnpm run pack` and `pnpm run test:packaged`: package and verify the installed app.

Installer output stays in root `dist/`. Generated output is ignored at every package
level. Release version preparation synchronizes all workspace manifests; release
verification checks each manifest against its own lockfile importer. Tag and publishing
rules remain in AGENTS.md.
