# KChess workspace

The workspace's one map; the other documents link here.

```text
apps/
  gpui/                  The app: the shell, Settings, the updater and the pages that change most
                         (analysis, Online, Players, Openings, the Computer page, …) and the
                         live Lichess game
    assets/              Boards, pieces, sounds and icons embedded in the app
    build/               App icons (written by `cargo xtask app-icon`)
    src/e2e/             End-to-end tests that drive the whole app headless
  cli/                   `kchess-cli`, the command line
crates/
  kchess-domain/         Chess, PGN, review, training and library document rules
  kchess-games/          Sessions as frontends drive them: computer, over-the-board, puzzle,
                         archive and Game history, and the round a broadcast opens at
  kchess-contracts/      `CoreApi`, its typed calls, its events and the serde types frontends decode
  kchess-core/           The headless core: storage, engines, Lichess, puzzles, reviews, voice
  kchess-client/         The core as a frontend holds it, without the core: the handle, the
                         service's protocol and socket, the profile's layout
  kchess-service/        One core per profile, served to every KChess window and the CLI, and the
                         core a frontend embeds (`embed`)
  kchess-voice/          The speech recognition helper (libvosk)
  kchess-delta/          What a release and an installed app agree on: versions, release notes,
                         asset and install names, the signed checksums and update patches
  kchess-host/           What the core and frontends read alike of their process: the `KCHESS_*`
                         switches, the App Store sandbox, log redaction, links a browser may open,
                         and the core's configuration (`config`)
  kchess-ui/             The app's design system over GPUI Kit: theme, motion, the line chart
                         (`docs/app-crates.md`: splitting the app into crates)
  kchess-app/            The app's services: the core as the app holds it, Settings, view
                         preferences, the log, a page's loading, quitting
  kchess-board/          The board and what every board page shares: game sessions, notation,
                         sounds, voice and the microphone, what pages know of the live game,
                         the puzzle board and database, exporting games, finding players
  kchess-nav/            Pages as the shell drives them: what a page declares, opening one from
                         another, the board pages' layout
  kchess-home/           The Home page
  kchess-history/        The History page
  kchess-database/       The Database page: a game database's games, searched and opened
  kchess-local/          The Over the board page and its chess clock
  kchess-practice/       The Practice page: drills, endgames, openings, your mistakes, Guess the move
  kchess-puzzles/        The Puzzles page: training, the daily puzzle, Puzzle Rush, stats, history
  kchess-studies/        The Studies page
  kchess-watch/          The Watch page: Lichess TV, followed players' games, broadcasts
  kchess-testkit/        The loopback HTTP/1.1 server the tests' and xtask's stand-ins answer through
  kchess-hack/           The workspace hack (`cargo hakari`): one feature set for every dependency
tooling/xtask/           `cargo xtask`: checks, the pre-push hook, release versions, packaging
fuzz/                    cargo-fuzz targets over the rules (its own workspace; `check` builds it)
vendor/gpui-pre/         GPUI with a window zoom (`KCHESS-PATCH.md`)
```

One Cargo workspace, one `Cargo.lock`, one version (the release, in `[workspace.package]`).

How a frontend reaches the core, and what each side owns, is in `CORE_FRONTENDS.md`; the rules
that hold it in place are AGENTS.md's "Architecture and regression ownership".

The chess and document rules are one Rust implementation (`crates/kchess-domain`), a library of
typed functions the core, the games, the app and the CLI call directly. `tests/rules_corpus.rs`
pins their answers through its own name-to-function table (`tests/corpus/`).

The app holds positions as FEN strings answered by the rules, keeps view preferences in the core
(`viewPreferences`) so a page opens as it was left, and owns only presentation: drawing, input,
sounds, notifications and the microphone.
