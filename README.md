# KChess

See [ARCHITECTURE.md](ARCHITECTURE.md) for the core, Node host, desktop and CLI workspace layout.

[![CI](https://github.com/ketansingrole/KChess-V2/actions/workflows/ci.yml/badge.svg)](https://github.com/ketansingrole/KChess-V2/actions/workflows/ci.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

KChess is a TypeScript desktop chess app built with Electron, Nuxt 4, Nuxt UI, and Tailwind CSS. Chess rules run in Rust (`crates/kchess-domain`, on [`shakmaty`](https://github.com/niklasf/shakmaty), with the semantics of [`chessops`](https://github.com/niklasf/chessops)), the board is [`@lichess-org/chessground`](https://github.com/lichess-org/chessground), game review is a custom PGN replay on the same board, and the Lichess API client is typed with Lichess's official [`@lichess-org/types`](https://github.com/lichess-org/api) OpenAPI types through `openapi-fetch`. Sounds and board themes are the Lichess assets. The rating chart uses [Unovis](https://unovis.dev), and IPC input is validated with [Valibot](https://valibot.dev). A headless Node core handles Stockfish, Lichess, local storage, and OAuth; Electron supplies the desktop shell.

## Core and frontends

Desktop and CLI share `crates/kchess-node/js` services and framework-independent game controllers in
`crates/kchess-wasm/js`. Electron is one host adapter; the Node host runs each profile in an isolated
worker, allowing several independent clients in the same process.

```bash
pnpm run cli -- --help
pnpm run cli -- library
pnpm run cli -- play beginner
pnpm run cli -- local
pnpm run cli -- call studyCommand '[{"op":"save","name":"Opening","pgn":"1. e4 e5 *"}]'
```

The CLI uses a separate profile at `~/.kchess/node`. Select another with `--data-dir`.
`login` opens Lichess OAuth; `--browser print` prints the URL for a headless host.
Credentials use macOS Keychain, Linux Secret Service (`secret-tool`), or Windows DPAPI.
`--no-credentials` disables credential storage. Desktop and Node ciphertext formats differ,
so use separate profiles rather than pointing the CLI at Electron's profile.

`pnpm run build:packages` builds the Node host and CLI under their `dist/` folders.
`pnpm run test:core` verifies the direct core, concurrent Node hosts and CLI in temporary
profiles without accessing your accounts or keychain. See
[core/frontend architecture](CORE_FRONTENDS.md) for host contracts,
shutdown ownership and examples.

## Run

Requires Node.js **24.21.0** (Node 24 LTS), pnpm 11.19.0 and [Rust](https://rustup.rs) via rustup. The runtime is pinned in `.nvmrc` and `.node-version`; use `nvm install && nvm use` if you use nvm. `rust-toolchain.toml` pins the Rust toolchain, which rustup installs on first use.

```bash
npm install --global pnpm@11.19.0
pnpm install --frozen-lockfile
pnpm run dev
```

`pnpm run dev` reuses a healthy existing session and starts one supervised Nuxt + Electron session otherwise. It recovers stale processes belonging to this checkout, prevents duplicate launches with a session lock, and stops its children on exit. An unrelated server occupying port 3000 produces an actionable error.

`pnpm run build` type-checks the renderer, Electron, and tests, generates the static renderer, and builds the main and preload bundles. Renderer assets and every packaged app are checked for nested apps/installers, redundant renderer dependencies, unused engine builds, and a 32 MiB app-payload budget (excluding Electron). `pnpm start` opens the latest build; `pnpm run app` builds first.

Voice input downloads the 40 MB Apache-2.0 English Vosk model from `alphacephei.com`
on first use, verifies its pinned SHA-256 digest, and caches it in the app's user-data
folder. Subsequent use works offline. Download progress and retry errors appear in the
voice control. Dev/build no longer downloads or bundles the model.
`pnpm run prepare:voice` prepares a verified test fixture in `.data/voice/cache/`;
voice e2e tests copy this fixture into their isolated profile to run offline.
These generated files are ignored by Git.

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

Audio is processed locally by Vosk WebAssembly in a worker, and is never uploaded or recorded by
KChess. Microphone tracks stop between runs/turns, when voice is disabled, and when leaving the
page. Disabling voice also releases the model. The implementation currently recognizes English;
accent/noise accuracy still needs evaluation with real speakers. Tests cover command parsing,
stale-result cancellation, microphone cleanup, and actual worker/model startup in Electron.

**Settings → Voice input** shows the microphone permission, has a live mic level test, and holds
the _Confirm spoken moves_, _Hold to speak_ and _Keep voice history_ preferences. The **voice
history** below them logs each phrase as text (never audio): what was heard with per-word
confidence, how it was read, what you meant (the square asked for, or the move you then played),
and the outcome. It lists the least certain words and the most common “heard → meant” mistakes,
and can be exported as JSON or cleared. On macOS, KChess asks the system for
microphone access the first time voice starts (Electron doesn't do this on its own). If access is
blocked, the voice panel offers **Open privacy settings** and resumes automatically once you allow
it. In a dev build started from a terminal (`pnpm run dev`), macOS attributes the microphone to the
app that launched KChess (your terminal or editor), so allow _that_ app under
Privacy & Security → Microphone.

| Command                                   | Purpose                                                                              |
| ----------------------------------------- | ------------------------------------------------------------------------------------ |
| `pnpm run check`                          | Formatting, ESLint, all type checks, smoke and unit tests                            |
| `pnpm test`                               | Chess, puzzle/database and main-process smoke tests, plus Vitest                     |
| `pnpm run test:unit`                      | Mounted Vue components, real Pinia stores, lifecycle and diagnostic regression tests |
| `pnpm run test:watch`                     | Vitest watch mode                                                                    |
| `pnpm run lint:fix` / `pnpm run format`   | Apply lint fixes / formatting                                                        |
| `pnpm run build && pnpm run test:e2e`     | Playwright against the built Electron app                                            |
| `pnpm run pack && pnpm run test:packaged` | Package for this platform and verify the packaged app                                |

UI tests launch through the branded wrapper, use a separate `.dev/automation` copy on macOS and a temporary profile, and never import legacy accounts or use your saved tokens. They cover SQLite and preload startup, bundled Stockfish, board moves, navigation, puzzle remounting, promotion and settings persistence. On headless Linux, run Electron checks through `xvfb-run --auto-servernum`. Failure traces and screenshots are written to `test-results/`.

CI runs `pnpm run check` and a desktop matrix on macOS, Windows and Linux that builds, exercises the UI, packages, and launches the packaged app. Dependabot checks npm dependencies weekly and GitHub Actions monthly.

Settings → Data & storage → **Export diagnostics** saves a JSON report containing apps/desktop/app/runtime versions and up to three bounded local log files. Tokens and OAuth fields are redacted; account and game databases are excluded. Logs live in the user-data directory's `logs/` folder. `KCHESS_USER_DATA_DIR` selects an isolated profile for testing or development and disables importing the previous Rust installation.

On macOS, dev and local runs launch through `tooling/with-branded-electron.mjs`, which keeps a `KChess.app` copy of Electron in `.dev/` (gitignored). It is renamed the way a packaged app is (bundle, executable, helper apps) and given the KChess icon, so the menu bar, Dock, Cmd-Tab, and Mission Control show "KChess" and its icon instead of Electron's. It is rebuilt automatically when Electron, the script, or `apps/desktop/build/icon.icns` changes; quit any running dev session first.

On macOS, `pnpm run pack:mac` creates an unsigned local Apple Silicon `.app` in `dist/mac-arm64/`.

### Releasing

Installers are built by `.github/workflows/release.yml`. Versions are dates with a counter,
`YEAR.MONTH.COUNTER`: the first release in October 2026 is `2026.10.0`, the next `2026.10.1`, and
the counter restarts each UTC month. There's no zero padding (`2026.10.0`, not `2026.10.00`), which keeps
the version valid semver. Follow the release checklist in [AGENTS.md](AGENTS.md).
Merge the intended changes with green CI first, then prepare the version change:

```bash
pnpm run release:version   # fetches origin's tags; updates package.json; dependencies stay locked
pnpm run release:verify
pnpm run check
git diff -- package.json pnpm-lock.yaml
git add package.json pnpm-lock.yaml
git commit -m "Release $(node -p "require('./package.json').version")"
```

Merge the reviewed version commit into `main` and wait for green CI. From a clean checkout
of that commit, create an annotated tag derived from its package version and push only
that tag, as shown in `AGENTS.md`. Never reuse or move a release tag. The version helper
stops if it cannot refresh remote tags; it does not guess using a stale local list.

The workflow builds on macOS, Windows and Linux and attaches the installers, plus `SHA256SUMS.txt`,
and updater manifests/blockmaps to a **draft** GitHub Release; review it and press _Publish_. A tag that isn't `vYEAR.MONTH.COUNTER`
or doesn't match `package.json` fails validation, as does
a release commit outside `main` or failing repository checks. Reruns can replace assets
on an existing draft, but refuse to overwrite a published release. Creating a release
prepares a draft; publishing it is a separate action.
_Run workflow_ in the Actions tab builds the same installers as downloadable artifacts
without creating a release, including when a tag is selected. Retry a draft release by
rerunning its original tag-push workflow.

| Platform | Files                                         |
| -------- | --------------------------------------------- |
| macOS    | `.dmg` and `.zip` for Apple Silicon (`arm64`) |
| Windows  | `.exe` installer (x64)                        |
| Linux    | `.AppImage` and `.deb` (x64)                  |

Without signing credentials, Windows releases are unsigned and macOS builds are ad-hoc signed (`tooling/release-config.cjs`), so on
first launch macOS says it can't verify the developer: open **System Settings → Privacy & Security**
and choose **Open Anyway**. Windows SmartScreen shows a similar warning (**More info → Run anyway**).

### App updates

**Settings → Updates** shows the installed version, update status, last successful check,
download progress, manual check/download/restart actions and a link to release notes.
By default, installed builds check stable GitHub releases ten seconds after startup and every
six hours, download updates in the background, and install when the user quits. Each behaviour
has its own switch. KChess never initiates a restart while you are playing; **Restart and install**
asks first. A normal quit with an update ready installs it without reopening the app.
Accounts, preferences and saved history remain in the existing user-data directory.
Development builds do not check for updates; unsupported unpacked installations cannot update themselves.

Windows NSIS, Linux AppImage and Debian installations support automatic installation
(Debian may require an operating-system authorization prompt). Ad-hoc signed Mac releases
check for updates and link to a manual download. [macOS automatic installation requires
Developer ID signing](https://www.electron.build/v26/docs/features/auto-update/).

To enable Mac automatic installation, add these GitHub Actions repository secrets:

- `MAC_CSC_LINK`: base64-encoded Developer ID Application `.p12` certificate.
- `MAC_CSC_KEY_PASSWORD`: its export password.
- `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID`: notarization credentials.

The release config imports the certificate, requires signing and notarization, and marks only
those Mac builds as able to install updates. Keep the same app ID and signing identity across
releases. Before uploading signed Mac packages, the workflow verifies their code signature,
signing team, Gatekeeper acceptance and stapled notarization ticket.
Never put signing credentials or a GitHub token in the app. Windows signing can be
added separately without changing the update feed.

Publish the complete release: installers, `latest.yml` (Windows), `latest-mac.yml` (Mac),
`latest-linux.yml` (Linux), and `.blockmap` files. The workflow verifies and attaches the manifests;
electron-builder generates the packaged `app-update.yml` from the fixed public GitHub provider.
Drafts and prereleases are excluded. Do not overwrite an already published release to ship a
fix: advance `YEAR.MONTH.COUNTER`, create a new tag, and publish its draft once all platforms
are ready. macOS requires the ZIP artifacts as well as DMGs.

**Bootstrap:** releases shipped before the updater was added cannot acquire it automatically.
Users must install the first updater-enabled release manually; subsequent releases use this flow.
An ad-hoc Mac installation also needs a manual upgrade to the first Developer ID signed release.
Before publishing broadly, verify a real version A → B update on installed copies of each
supported platform, including restart, normal quit, opt-out, offline retry and data preservation.
Mocked tests and local unsigned builds cannot verify signing or the published feed end to end.

## Features

- Dashboard with public Lichess profile ratings, rating history when Lichess provides it, and recent games.
- Computer games at low, medium, and high Stockfish strengths, with move history, replay, takeback, board flip, promotion picker, and draw detection (repetition and the 50-move rule).
- Live Lichess play via OAuth, public seeks, direct challenges, clocks, resign, and takeback offers. Connect as many of your own Lichess accounts as you like and choose which one plays (from the sidebar menu or the Play Online page); each has its own encrypted token.
- Puzzles, with every screen labelled by where its data lives (see below): rated training on any theme and difficulty (the result changes your Lichess puzzle rating), practice and offline training that send nothing, the daily puzzle, your Lichess puzzle rating, history and per-theme strengths, and Storm, Streak and Rush played locally.
- Practice drills, all local: board coordinates (find or name the square), square colours, knight paths, endgame drills against Stockfish from positions checked to be won or drawn, and puzzle themes such as checkmate patterns.
- Synced Lichess history reviewed in a built-in PGN replay (move list, keyboard navigation).
- Lichess board themes and 11 open-licensed piece sets with adjustable piece animation, 22 app color themes with separate choices for light and dark mode, plus your own themes as JSON files in a themes folder, alerts for game events (opponent moves, low time, game start and result, computer moves) shown in the app while you use it and as system notifications while it is in the background, each switchable, board coordinates (inside / outside / hidden), Lichess move sounds, appearance settings, account management, and a choice of Stockfish in Settings: the bundled Stockfish 19 (the [`stockfish`](https://www.npmjs.com/package/stockfish) npm package's lite multi-threaded WASM build, run as a UCI process by Electron's Node runtime, so it works offline on every platform), a native build you can download (macOS; verified against the SHA-256 digest GitHub publishes for the release) and delete again from Settings, or any executable you pick yourself.
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

On first launch on macOS, the app imports accounts, game history, settings, and available tokens from a previous Rust KChess installation at `~/.kchess/kchess.db`, when present. The old database is left untouched. New app data is stored in SQLite (`kchess.db` in Electron's user-data directory, via Node's built-in `node:sqlite`; schema changes are ordered migrations in `crates/kchess-core/src/store/migrations.rs`, tracked with `PRAGMA user_version`); existing `kchess-data.json` / `lichess-tokens.json` files are imported once and archived as `.bak`. OAuth tokens are encrypted with Electron `safeStorage`.

## Structure

- `apps/desktop/app/pages/` contains separate Dashboard, Online, Computer, Puzzles, Practice, History, Friends, and Settings routes (Settings shows one category at a time, chosen from the sidebar).
- `apps/desktop/app/stores/` holds the Pinia stores that keep game and account state alive while navigating: `kchess.ts` (navigation, accounts, settings), with `kchess/computerGame.ts`, `onlineGame.ts` and `gameHistory.ts` owning their respective game state, `friends.ts` (followed players), `puzzles.ts` (puzzle training, Lichess puzzle data, the local puzzle database), and `usage.ts` (data downloaded and stored). `apps/desktop/app/components/` holds reusable board, game row, and page header components.
- `apps/desktop/app/assets/` and `apps/desktop/app/utils/` hold styles, media and presentation helpers (board pieces, sounds, image export, swipe navigation), and `utils/library.ts`, which loads the core's library before the app mounts.
- `crates/kchess-node/js/` is the headless core: chess engine, Lichess, puzzles, review and the local library (studies, played games, mistake drills, unfinished sessions). `createKChessCore(platform)` implements `CoreApi` and emits `CoreEvents`; it never imports Electron. The host supplies a `CorePlatform` with its data directory, token encryption, browser opening and power state. `pnpm run build:packages` builds it as a plain Node library in `core/dist`.
- `apps/desktop/electron/main/` and `apps/desktop/electron/preload/` are the Electron desktop shell. Main configures the core with an Electron platform, serves every `CoreApi` method and event over IPC, and adds the window, updater, dialogs, notifications, themes, microphone access and the `kchess://` route that serves the core's voice model.
- `crates/kchess-wasm/js/` contains pure chess, training and document rules. `crates/kchess-contracts/ts/` owns the headless API, events, types and validation tuples. Desktop IPC and diagnostics contracts live in `apps/desktop/contracts/`.
- `apps/desktop/nuxt.config.ts` enables client rendering and hash routing, so the generated app works from Electron's local file URL without a running server. Nuxt's generated files live in `apps/desktop/.output/public/` and are included in the macOS package.

Nuxt UI is loaded as a Nuxt module. Desktop APIs remain exposed through Electron's preload bridge at `window.kchess`.

## Security notes

- The renderer is sandboxed with context isolation; the main process validates all IPC input and never lets the renderer choose which executable to run: an engine path must come from the native file picker or be the copy KChess downloaded (the bundled engine needs no path). Deleting only ever removes KChess's own download folder.
- The puzzle database is downloaded from a fixed URL only, and only after a click; the renderer supplies no URL or path for it.
- The Lichess OAuth loopback server only accepts the `/callback` request that carries the expected `state`.
- The window cannot navigate away from the app's own pages.

## License

KChess is free software, released under the **GNU General Public License v3.0 or later** (see [`LICENSE`](LICENSE)). You may use, study, change, and share it under those terms; anything you distribute that is based on it must stay under the GPL and come with its source.

This is required, not just chosen: the app builds on copyleft code and assets from the Lichess and Stockfish projects.

- `@lichess-org/chessground`, `shakmaty` and the parts of `chessops` ported to Rust — GPL-3.0-or-later
- `stockfish` (the bundled WASM engine) — GPL-3.0
- `@lichess-org/types` (type definitions only, nothing of it ships at runtime) — AGPL-3.0-or-later
- Lichess board themes, sound effects and some piece sets — AGPLv3+ (other piece sets are GPL, MIT, Apache-2.0 or CC0; see the attribution file)
- The rest are permissively licensed (MIT, ISC, Apache-2.0, BlueOak).

The full list, with what each part is used for, is in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md); asset credits are in [`apps/desktop/app/assets/ATTRIBUTION.md`](apps/desktop/app/assets/ATTRIBUTION.md). The app icon is original artwork drawn by [`tooling/make-icons.py`](tooling/make-icons.py) and covered by the same license.

## Not affiliated

KChess is an independent project. It is **not affiliated with, endorsed by, or sponsored by** [Lichess](https://lichess.org) or the [Stockfish](https://stockfishchess.org) team. It talks to Lichess through Lichess's public API and OAuth, and uses its open-source assets under their licenses. Bulk downloads (game sync, profiles) run one request at a time, as Lichess asks; during a live game it checks connection status about every 3 seconds, and every request is counted in Settings → Data & storage. "Lichess" and "Stockfish" are names of their respective projects.

## Contributing and security

Contributions are welcome; see [`CONTRIBUTING.md`](CONTRIBUTING.md). To report a vulnerability, follow [`SECURITY.md`](SECURITY.md) rather than opening a public issue.
