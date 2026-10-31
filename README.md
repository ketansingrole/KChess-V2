# KChess

[![CI](https://github.com/ketansingrole/KChess-V2/actions/workflows/ci.yml/badge.svg)](https://github.com/ketansingrole/KChess-V2/actions/workflows/ci.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

KChess is a desktop chess app written in Rust: play Stockfish, play on Lichess, follow friends, review games, solve puzzles and train. The window is [GPUI](https://github.com/zed-industries/zed) with [GPUI Kit](https://github.com/longbridge/gpui-kit); chess rules run on [`shakmaty`](https://github.com/niklasf/shakmaty) (`crates/kchess-domain`); a headless core (`crates/kchess-core`) handles Stockfish, Lichess, the local library, puzzles and OAuth. Sounds and board themes are the Lichess assets.

## Apps

| App          | Where       | Run in development                    | Package              |
| ------------ | ----------- | ------------------------------------- | -------------------- |
| KChess       | `apps/gpui` | `cargo run -p kchess-gpui`            | `cargo xtask package` |
| Command line | `apps/cli`  | `cargo run -p kchess-cli -- --help`   | ships with the app    |

## Run

### Prerequisites

- Rust through [rustup](https://rustup.rs); the toolchain is pinned in `rust-toolchain.toml`.
- On Linux, the libraries the app builds against:
  `sudo apt-get install libasound2-dev libfontconfig-dev libwayland-dev libx11-xcb-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libvulkan-dev`.

`cargo xtask install-hooks` installs the Git pre-push hook (see [GUARDRAILS.md](GUARDRAILS.md)).

### The app

KChess opens your real profile by default, with your games, settings and accounts. For
development, point it at a separate profile and at a native Stockfish (development builds ship
none):

```bash
KCHESS_ENGINE_PATH="$(cargo run -q --release -p kchess-core --example fetch_stockfish -- .data/stockfish)"
export KCHESS_ENGINE_PATH
KCHESS_USER_DATA_DIR=.data/gpui-profile cargo run -p kchess-gpui
```

`fetch_stockfish` downloads and verifies the latest official Stockfish into the folder once and
prints its path. Add `--release` to `cargo run` for a fast build; development builds show a
**Dev** badge in the sidebar, release builds never do. `cargo xtask voice-runtime debug` stages the
speech helper and libvosk for voice input in a debug build.

To see changes as you work, let KChess rebuild and restart itself on every save:

```bash
cargo xtask app --watch
```

It runs on the scratch profile `.data/gpui-profile` with a native Stockfish, fetched once, and
watches `apps/` and `crates/`. The app comes back on the page you were on, and stays open on the
last good build when one fails. The core service restarts only when its own code changed. Stop
with Ctrl-C or `cargo xtask app --stop`; the app's output is in `.data/gpui.log`.

Several KChess windows, and the command line, share one profile through its core service
([docs/shared-core.md](docs/shared-core.md)). The app uses it when `kchess-service` is built beside
it (`cargo build -p kchess-service`); `KCHESS_CORE_MODE=embedded` or `service` chooses explicitly.

On macOS, `cargo run` re-signs the app with your Apple Development identity and a fixed
identifier (`tooling/macos-dev-runner.sh`), so the keychain's "Always Allow" for the profile's
credential key lasts across rebuilds. Choose another identity with `KCHESS_DEV_SIGNING_IDENTITY`,
or `-` to skip signing.

A profile last used by the Electron app that KChess once was (any release up to 2026.10.15) is
taken over as it is: its studies, games and sessions, page settings, and Lichess logins
(`crates/kchess-electron-legacy`).

### Command line

```bash
cargo run -p kchess-cli -- --help
cargo run -p kchess-cli -- library
cargo run -p kchess-cli -- play beginner
cargo run -p kchess-cli -- local
cargo run -p kchess-cli -- call studyCommand '[{"op":"save","name":"Opening","pgn":"1. e4 e5 *"}]'
```

`kchess-cli` uses the app's profile (or `--data-dir`) through the profile's core service, so it
works while the app is open; `--embedded` runs a core in its own process instead. Packages ship
it beside the app (`/usr/bin/kchess-cli` from the `.deb`).

### Packaging

```bash
cargo xtask package             # KChess for this platform, with the update manifests (apps/gpui/dist/)
```

macOS makes a `.dmg` and `.zip` (Apple Silicon), Windows a per-user NSIS installer, Linux an
AppImage and a `.deb` (`--snap` makes the Snap). Each carries the app's own Stockfish, the core
service, the command line, the voice runtime and `THIRD_PARTY_LICENSES.txt`. `--skip-build`
reuses the release binaries already built. Release packages come from the Release workflow.

### Tests and checks

| Command                  | What it does                                                              |
| ------------------------ | ------------------------------------------------------------------------- |
| `cargo xtask check-fast` | Format, lints and the tests of the packages changed since main            |
| `cargo xtask check`      | Everything a push to main must pass (runs from the pre-push hook)         |
| `cargo test -p kchess-gpui e2e::` | The app's end-to-end tests, headless over a fresh profile        |
| `cargo xtask help`       | The rest: release versions, notices, licenses, the voice runtime          |

[GUARDRAILS.md](GUARDRAILS.md) describes the verification ladder. CI runs `cargo xtask check`,
`cargo deny`, and the app's tests and a package on macOS, Windows and Linux.

### Voice input

In **Play with Computer**, turn on voice with the **Voice** button under the board and allow
microphone access. Say “E two to E four”, “Knight F three”, “Queen takes D five”, “pawn to E four”
or “castle kingside”. Phonetic file names (Alpha through Hotel) also work, for example “Echo two to
Echo four”. Spoken moves require confirmation by default: say “confirm” or select the move.
Ambiguous moves and unspecified promotions show numbered choices; say the number or select one.
Say “cancel” to discard a pending move. A move heard with low confidence always asks
“Did you mean …?” before it is played. Ordinary board input continues to work.

Game commands work at any time, not only on your turn: “take back” (or “undo”), “new game”,
“resign” and “switch sides” (or “flip board”). New game, resign and switching sides mid-game ask
first; answer by saying “confirm” or “cancel”. Moves are only taken on your turn.

In **Practice → Coordinates → Say the square**, enable voice input, then start a run and name
the highlighted square (“E four” or “Echo four”). Recognition is prepared before the timer starts.
These scores are stored separately from keyboard training. Unrecognized or uncertain speech asks
for a repeat; a recognized incorrect square counts as a mistake.

Audio is processed locally by the Vosk library in a separate helper process, and is never uploaded or recorded by
KChess. Microphone tracks stop between runs/turns, when voice is disabled, and when leaving the
page. Disabling voice also releases the model. The implementation currently recognizes English;
accent/noise accuracy still needs evaluation with real speakers. Tests cover command parsing,
stale-result cancellation, microphone cleanup, and the helper and model starting for real.

**Settings → Voice input** shows the microphone permission, has a live mic level test, and holds
the _Confirm spoken moves_, _Hold to speak_ and _Keep voice history_ preferences. The **voice
history** below them logs each phrase as text (never audio): what was heard with per-word
confidence, how it was read, what you meant (the square asked for, or the move you then played),
and the outcome. It lists the least certain words and the most common “heard → meant” mistakes,
and can be exported as JSON or cleared. On macOS, KChess asks the system for
microphone access the first time voice starts. If access is
blocked, the voice panel offers **Open privacy settings** and resumes automatically once you allow
it. In a dev build started from a terminal (`cargo run`), macOS attributes the microphone to the
app that launched KChess (your terminal or editor), so allow _that_ app under
Privacy & Security → Microphone.

Voice input downloads the 40 MB Apache-2.0 English Vosk model from `alphacephei.com`
on first use, verifies its pinned SHA-256 digest, and caches it in the app's user-data
folder. Subsequent use works offline. Download progress and retry errors appear in the
voice control. Builds never download or bundle the model.

### Diagnostics

Settings → Data & storage → **Export diagnostics** saves a report containing the app and runtime versions and up to three bounded local log files. Tokens and OAuth fields are redacted; account and game databases are excluded. Logs live in the user-data directory's `logs/` folder. `KCHESS_USER_DATA_DIR` selects an isolated profile for testing or development and disables importing the previous Rust installation.

### Releasing

Installers come from `.github/workflows/release.yml` as a draft GitHub Release, versioned
`YEAR.MONTH.COUNTER`. Release steps, the Snap Store listing, signing and the app updater are in
[docs/releasing.md](docs/releasing.md); the checklist is in [AGENTS.md](AGENTS.md).

## Features

- Dashboard with public Lichess profile ratings, rating history when Lichess provides it, and recent games.
- Computer games at low, medium, and high Stockfish strengths, with move history, replay, takeback, board flip, promotion picker, and draw detection (repetition and the 50-move rule).
- Live Lichess play via OAuth, public seeks, direct challenges, clocks, resign, and takeback offers. Connect as many of your own Lichess accounts as you like and choose which one plays (from the sidebar menu or the Play Online page); each has its own encrypted token.
- Puzzles, with every screen labelled by where its data lives (see below): rated training on any theme and difficulty (the result changes your Lichess puzzle rating), practice and offline training that send nothing, the daily puzzle, your Lichess puzzle rating, history and per-theme strengths, and Storm, Streak and Rush played locally.
- Practice drills, all local: board coordinates (find or name the square), square colours, knight paths, endgame drills against Stockfish from positions checked to be won or drawn, and puzzle themes such as checkmate patterns.
- Synced Lichess history reviewed in a built-in PGN replay (move list, keyboard navigation).
- Lichess board themes and 11 open-licensed piece sets with adjustable piece animation, 22 app color themes with separate choices for light and dark mode, plus your own themes as JSON files in a themes folder, alerts for game events (opponent moves, low time, game start and result, computer moves) shown in the app while you use it and as system notifications while it is in the background, each switchable, board coordinates (inside / outside / hidden), Lichess move sounds, appearance settings, account management, and a choice of Stockfish in Settings: the official native Stockfish that ships beside the app (so it works offline on every platform), a newer official build you can download (verified against the SHA-256 digest GitHub publishes for the release) and delete again from Settings, or any executable you pick yourself.
- Friends: follow other players (add by name, or import who your connected accounts follow on Lichess), see their ratings and whether they are online, and keep their games for offline browsing. Each friend shows exactly how much data was downloaded and stored for them, and can be cleared or removed; removals are remembered.
- Data transparency: every request to Lichess is counted per account and purpose, and Settings → Data & storage shows what was downloaded and what is kept on disk.
- Board niceties: premoves, optional auto-queen, optional legal-move dots, drawing arrows with right-click drag, and arrow-key move navigation in every game.
- `Cmd+K` (or `Ctrl+K`) for the command palette: navigate, sync, or start a game (against Stockfish, or an online game, or challenge a friend). `Cmd+,` opens Settings.
- Online extras: draw offers, chat, incoming challenges with accept/decline, rematch, claim victory, berserk, correspondence games, variants (Chess960, King of the Hill, Three-check, Antichess, Atomic, Horde, Racing Kings) and tournaments (arenas and your teams' Swiss events).
- Watch Lichess TV, friends' live games and broadcasts; look up any player's ratings, records and your score against them.
- Analysis extras: Masters and Player explorer databases with filters, opt-in Lichess cloud evaluation, opening names, Lichess study import and export, and GIF/PNG/PGN export.
- Computer games from Chess960 or any position, with clocks; two players at one computer, a chess clock for real boards, blindfold and zen mode; an opening trainer; and Insights from your synced games.
- What KChess cannot do that Lichess does, and why, is in [`docs/lichess-parity.md`](docs/lichess-parity.md).

### What is synced with Lichess and what is not

Puzzles mix three kinds of data, and the app marks each one with a badge so it is never a guess:

| Badge                        | Meaning                                             | What it covers                                                                                                                                      |
| ---------------------------- | --------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Synced to Lichess**        | Doing it here changes your Lichess account          | Rated puzzle results (`POST /api/puzzle/batch`, needs the `puzzle:write` permission)                                                                |
| **From Lichess · read-only** | Fetched from Lichess; KChess never edits it         | Puzzle rating and history, the puzzle dashboard, the Storm dashboard, the daily puzzle                                                              |
| **Local only**               | Stays on this computer and is never sent to Lichess | Storm, Streak and Rush runs and scores, practice and offline puzzles, retrying a puzzle from history, the daily puzzle result, every practice drill |

Lichess has no way for an app to submit Storm, Streak or Racer results, so those modes are played locally against the puzzle database and never count toward your Lichess Storm or Streak numbers; your Lichess Storm and Streak results are shown next to them, read-only. Rush is KChess's own mode and has no Lichess counterpart. Puzzle Racer is not included.

Puzzle access needs the `puzzle:read` and `puzzle:write` permissions. Accounts connected before puzzles were added are asked to connect again the first time they need them.

### The local puzzle database

Storm, Streak, Rush and offline puzzles play from Lichess's public puzzle database (CC0). It is an opt-in download of about 300 MB from `database.lichess.org` (Settings → Data & storage, or the Rush tab). The file is streamed and decompressed without being saved: KChess keeps a random sample of around a hundred thousand well-tested puzzles, spread evenly over ratings and themes, in `kchess.db` (a few tens of MB), and can delete it again at any time. Local scores live in the same file and can be cleared separately.

On first launch on macOS, the app imports accounts, game history, settings, and available tokens from a previous Rust KChess installation at `~/.kchess/kchess.db`, when present. The old database is left untouched. App data is stored in SQLite (`kchess.db` in the profile folder; schema changes are ordered migrations in `crates/kchess-core/src/store/migrations.rs`, tracked with `PRAGMA user_version`); existing `kchess-data.json` / `lichess-tokens.json` files are imported once and archived as `.bak`. Lichess tokens are sealed with AES-256-GCM under a key the system's credential store keeps (Keychain, Credential Manager, Secret Service).

## Structure

- `apps/gpui/` is the app: its pages (`src/pages`), the board (`src/board`), the shell, settings,
  voice input, the updater and its end-to-end tests (`src/e2e`).
- `apps/cli/` is the command line.
- `crates/kchess-core/` is the headless core: chess engines, Lichess, puzzles, reviews and the
  local library (studies, played games, mistake drills, unfinished sessions), behind `CoreApi`
  (`crates/kchess-contracts`) and its events. `crates/kchess-service/` serves one core per profile
  to every KChess window and the command line.
- `crates/kchess-domain/` holds the pure chess, training and document rules; `crates/kchess-games/`
  the computer, over-the-board and Game history sessions as frontends drive them;
  `crates/kchess-voice/` the speech recognition helper; `crates/kchess-electron-legacy/` reads what
  the Electron app left in a profile.
- `tooling/xtask/` is the repository's tooling (`cargo xtask`); `fuzz/` the cargo-fuzz targets.

## Security notes

- The app never lets typed or downloaded data choose which executable runs: an engine path comes
  from the file picker or is the copy KChess downloaded. Deleting only ever removes KChess's own
  download folder.
- The puzzle database is downloaded from a fixed URL only, and only after a click.
- The Lichess OAuth loopback server only accepts the `/callback` request that carries the expected `state`.
- The core service's socket admits only clients of the same user with the profile's token.

## License

KChess is free software, released under the **GNU General Public License v3.0 or later** (see [`LICENSE`](LICENSE)). You may use, study, change, and share it under those terms; anything you distribute that is based on it must stay under the GPL and come with its source.

This is required, not just chosen: the app builds on copyleft code and assets from the Lichess and Stockfish projects.

- `@lichess-org/chessground`, `shakmaty` and the parts of `chessops` ported to Rust — GPL-3.0-or-later
- `stockfish` (the bundled WASM engine) — GPL-3.0
- `@lichess-org/types` (type definitions only, nothing of it ships at runtime) — AGPL-3.0-or-later
- Lichess board themes, sound effects and some piece sets — AGPLv3+ (other piece sets are GPL, MIT, Apache-2.0 or CC0; see the attribution file)
- The rest are permissively licensed (MIT, ISC, Apache-2.0, BlueOak).

The full list, with what each part is used for, is in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md); asset credits are in [`apps/gpui/assets/ATTRIBUTION.md`](apps/gpui/assets/ATTRIBUTION.md). The app icon is original artwork drawn by `cargo xtask app-icon` ([`tooling/xtask/src/app_icon.rs`](tooling/xtask/src/app_icon.rs)) and covered by the same license.

## Not affiliated

KChess is an independent project. It is **not affiliated with, endorsed by, or sponsored by** [Lichess](https://lichess.org) or the [Stockfish](https://stockfishchess.org) team. It talks to Lichess through Lichess's public API and OAuth, and uses its open-source assets under their licenses. Bulk downloads (game sync, profiles) run one request at a time, as Lichess asks; during a live game it checks connection status about every 3 seconds, and every request is counted in Settings → Data & storage. "Lichess" and "Stockfish" are names of their respective projects.

## Contributing and security

Contributions are welcome; see [`CONTRIBUTING.md`](CONTRIBUTING.md). To report a vulnerability, follow [`SECURITY.md`](SECURITY.md) rather than opening a public issue.
