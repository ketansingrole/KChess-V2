# KChess workspace

```text
core/                    Headless chess library
  src/domain/            Pure chess, game, training and document rules
  src/contracts/         Core API, events, types and argument validation
  src/services/          Engines, networking, credentials and persistence
  tests/unit/
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

Desktop IPC contracts extend the core validation tuples. Core validates direct calls
without loading desktop contracts, and desktop replaces the settings tuple with its
complete frontend settings shape. Import rules enforce these boundaries.

Run the existing root commands:

- `pnpm run dev`: one branded Electron + Nuxt development session.
- `pnpm run cli -- --help`: build the headless packages and run CLI.
- `pnpm run build:core`: build core, Node host and CLI into their own `dist/` folders.
- `pnpm --filter @kchess/core build`: build core alone, including declarations.
- `pnpm --filter @kchess/node build`: build the reusable Node host and declarations.
- `pnpm run build`: typecheck and build desktop into `apps/desktop/out/` and `.output/`.
- `pnpm run check:fast`: checks for editing; `pnpm run check`: full repository checks.
- `pnpm run test:e2e`: production desktop regressions with isolated profiles.
- `pnpm run pack` and `pnpm run test:packaged`: package and verify the installed app.

Installer output stays in root `dist/`. Generated output is ignored at every package
level. Release version preparation synchronizes all workspace manifests; release
verification checks each manifest against its own lockfile importer. Tag and publishing
rules remain in AGENTS.md.
