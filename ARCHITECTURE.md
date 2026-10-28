# KChess workspace

```text
apps/
  gpui/                  The app: GPUI Kit views, the board, shell, settings, voice, updater
    assets/              Boards, pieces, sounds and icons embedded in the app
    build/               App icons (written by `cargo xtask app-icon`)
    src/e2e/             End-to-end tests that drive the whole app headless
  cli/                   `kchess-cli`, the command line
crates/
  kchess-domain/         Chess, PGN, review, training and library document rules (`api.rs`)
  kchess-games/          Computer, over-the-board and Game history sessions as frontends drive them
  kchess-contracts/      `CoreApi`, its events and the serde types frontends decode
  kchess-core/           The headless core: storage, engines, Lichess, puzzles, reviews, voice
  kchess-service/        One core per profile, served to every KChess window and the CLI
  kchess-voice/          The speech recognition helper (libvosk)
  kchess-electron-legacy/ What the Electron app left in a profile, read once by the core
tooling/xtask/           `cargo xtask`: checks, the pre-push hook, release versions, packaging
fuzz/                    cargo-fuzz targets
vendor/gpui-pre/         GPUI with a window zoom (`KCHESS-PATCH.md`)
```

One Cargo workspace, one `Cargo.lock`, one version (the release, in `[workspace.package]`).

The core never depends on a frontend. Frontends call it by `CoreApi` method name with JSON
arguments and subscribe to its events; the profile's core service (`docs/shared-core.md`) lets
several of them share one core, and either can embed a core instead. Host capabilities a frontend
answers (opening the browser, focusing its window, battery state) reach it as `host:request`.

The chess and document rules are one Rust implementation (`crates/kchess-domain`), exposed through
`api.rs` as `call(method, argsJson)` and as plain functions; the core and the app use them
directly. `tests/rules_corpus.rs` pins their answers.

The app holds positions as FEN strings answered by the rules, keeps view preferences in the core
(`viewPreferences`) so a page opens as it was left, and owns only presentation: drawing, input,
sounds, notifications and the microphone.
