# KChess workspace

```text
crates/                  The headless core in Rust, built with Cargo (`Cargo.toml` workspace)
  kchess-domain/         Chess, PGN, review and library document rules (chessops semantics)
  kchess-core/           Services: storage, engines, Lichess, puzzles and logging (tokio)
  kchess-contracts/      Contract types generated from Rust; `ts/` is `@kchess/contracts`
  kchess-node/           `@kchess/native`: the N-API module and the Node binding (`js/`)
  kchess-wasm/           The same rules as WebAssembly for the renderer; `js/` is `@kchess/rules`
tests/core/              Unit suites for the packages above
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

The native package never imports a host or application. `@kchess/rules` and `@kchess/contracts`
remain pure and cannot import Node, Electron or frontend frameworks. Frontends import the rules
and contracts by name (`@kchess/rules/*`, `@kchess/contracts/*`); privileged services are
available through `@kchess/native` and `@kchess/native/logger` only.
Electron implements `CorePlatform`; the reusable Node host supplies worker isolation,
OS credentials and browser login. CLI owns terminal input and presentation.

The chess and document rules are Rust (`crates/kchess-domain`), one implementation for every
runtime. `crates/kchess-domain/src/api.rs` exposes them as `invoke(method, argsJson)`; the core
loads it as a Node module (`@kchess/native`, required, no TypeScript fallback) and the renderer
as WebAssembly (`crates/kchess-wasm`, loaded by `apps/desktop/app/plugins/rules.client.ts`
before the app mounts). `crates/kchess-wasm/js/engine.ts` holds the binding each host sets, and the
domain functions (`treeFromPgn`, `addMove`, `replay`, `analyseReview`, `setupPgn` …) are thin
wrappers over it. Core services use `crates/kchess-node/js/rules.ts`, which also covers the
library decoders, review summaries, Lichess lines and the puzzle sampler.

Frontends hold positions as `Position` values (`crates/kchess-wasm/js/position.ts`): a variant and a
FEN, immutable, with every question (destinations, check, outcome, playing a move) answered by
the Rust rules. Positions can live in reactive state and cross IPC as plain setups. Every rule,
positions included, is pinned to `tests/core/native-golden.json`: the outputs of the
chessops-based TypeScript rules they replaced, for seeded cases that the tests generate with the
Rust rules themselves (`KCHESS_FUZZ_SCALE=20` searches deeper, beyond the recorded cases); the
same tests run the WebAssembly module against the Node module. Contracts check the shape of a saved game or study command; the core service
validates it in Rust. The deliberate differences from chessops, both reachable only from a typed FEN: an en
passant square that a piece occupies is dropped rather than kept, and Racing Kings refuses both
kings on the goal with Black to move. Review figures go
through `exp`, which the Rust rules compute as V8 does (fdlibm, with the multiply-adds V8's
compiler fuses on arm64); the WebAssembly loader compares both variants with `Math.exp` and
keeps the one that matches.

Desktop IPC contracts extend the core validation tuples. Core validates direct calls
without loading desktop contracts, and desktop replaces the settings tuple with its
complete frontend settings shape. Import rules enforce these boundaries.

Run the existing root commands:

- `pnpm run dev`: one branded Electron + Nuxt development session.
- `pnpm run cli -- --help`: build the headless packages and run CLI.
- `pnpm run build:packages`: build the Node host and CLI into their own `dist/` folders.
- `pnpm run build:native`: build the Rust rules (Node module and WebAssembly) and the three
  TypeScript packages' `dist/` (`@kchess/contracts`, `@kchess/rules`, `@kchess/native`); part of `build` and `dev`.
- `pnpm --filter @kchess/node build`: build the reusable Node host and declarations.
- `pnpm run check:rust`: rustfmt, clippy and Rust unit tests.
- `pnpm run build`: typecheck and build desktop into `apps/desktop/out/` and `.output/`.
- `pnpm run check:fast`: checks for editing; `pnpm run check`: full repository checks.
- `pnpm run test:e2e`: production desktop regressions with isolated profiles.
- `pnpm run pack` and `pnpm run test:packaged`: package and verify the installed app.

Installer output stays in root `dist/`. Generated output is ignored at every package
level. Release version preparation synchronizes all workspace manifests; release
verification checks each manifest against its own lockfile importer. Tag and publishing
rules remain in AGENTS.md.
