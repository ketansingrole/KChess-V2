# KChess

[![CI](https://github.com/ketansingrole/KChess-V2/actions/workflows/ci.yml/badge.svg)](https://github.com/ketansingrole/KChess-V2/actions/workflows/ci.yml)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-blue.svg)](LICENSE)

KChess is a TypeScript desktop chess app built with Electron, Nuxt 4, Nuxt UI, and Tailwind CSS. Chess rules use [`chessops`](https://github.com/niklasf/chessops), the board is [`@lichess-org/chessground`](https://github.com/lichess-org/chessground), game review is a custom PGN replay built on `chessops` and the same board, and the Lichess API client is typed with Lichess's official [`@lichess-org/types`](https://github.com/lichess-org/api) OpenAPI types through `openapi-fetch`. Sounds and board themes are the Lichess assets. The rating chart uses [Unovis](https://unovis.dev), and IPC input is validated with [Valibot](https://valibot.dev). The Electron main process handles Stockfish, Lichess, local storage, and OAuth.

## Run

Requires Node.js 22.12+ and npm.

```bash
npm install
npm run dev
```

`npm run dev` runs Nuxt and Electron together. `npm run build` type-checks both sides, generates the static Nuxt renderer, and builds Electron's main and preload bundles. `npm start` opens the built app. `npm test` runs the chess-logic and main-process smoke tests (`test:chess` runs just the chess part). `npm run format:check` and `npm run typecheck` run in CI (`.github/workflows/ci.yml`).

On macOS, dev and local runs launch through `scripts/with-branded-electron.mjs`, which keeps a `KChess.app` copy of Electron in `.dev/` (gitignored). It is renamed the way a packaged app is (bundle, executable, helper apps) and given the KChess icon, so the menu bar, Dock, Cmd-Tab, and Mission Control show "KChess" and its icon instead of Electron's. It is rebuilt automatically when Electron, the script, or `build/icon.icns` changes; quit any running dev session first.

On macOS, `npm run pack:mac` creates an unsigned local `.app` in `dist/mac-arm64/` (or the matching architecture directory).

## Features

- Dashboard with public Lichess profile ratings, rating history when Lichess provides it, and recent games.
- Computer games at low, medium, and high Stockfish strengths, with move history, replay, takeback, board flip, promotion picker, and draw detection (repetition and the 50-move rule).
- Live Lichess play via OAuth, public seeks, direct challenges, clocks, resign, and takeback offers. Connect as many of your own Lichess accounts as you like and choose which one plays (from the sidebar menu or the Play Online page); each has its own encrypted token.
- Synced Lichess history reviewed in a built-in PGN replay (move list, keyboard navigation).
- Lichess board themes and 11 open-licensed piece sets with adjustable piece animation, 22 app color themes with separate choices for light and dark mode, plus your own themes as JSON files in a themes folder, alerts for game events (opponent moves, low time, game start and result, computer moves) shown in the app while you use it and as system notifications while it is in the background, each switchable, board coordinates (inside / outside / hidden), Lichess move sounds, appearance settings, account management, and a choice of Stockfish in Settings: the bundled Stockfish 19 (the [`stockfish`](https://www.npmjs.com/package/stockfish) npm package's lite multi-threaded WASM build, run as a UCI process by Electron's Node runtime, so it works offline on every platform), a native build you can download (macOS; verified against the SHA-256 digest GitHub publishes for the release) and delete again from Settings, or any executable you pick yourself.
- Friends: follow other players (add by name, or import who your connected accounts follow on Lichess), see their ratings and whether they are online, and keep their games for offline browsing. Each friend shows exactly how much data was downloaded and stored for them, and can be cleared or removed; removals are remembered.
- Data transparency: every request to Lichess is counted per account and purpose, and Settings → Data & storage shows what was downloaded and what is kept on disk.
- Board niceties: premoves, optional auto-queen, optional legal-move dots, drawing arrows with right-click drag, and arrow-key move navigation in every game.
- `Cmd+K` (or `Ctrl+K`) for the command palette: navigate, sync, or start a game (against Stockfish, or an online game, or challenge a friend). `Cmd+,` opens Settings.

On first launch on macOS, the app imports accounts, game history, settings, and available tokens from a previous Rust KChess installation at `~/.kchess/kchess.db`, when present. The old database is left untouched. New app data is stored in SQLite (`kchess.db` in Electron's user-data directory, via Node's built-in `node:sqlite`; schema changes are ordered migrations in `src/main/migrations.ts`, tracked with `PRAGMA user_version`); existing `kchess-data.json` / `lichess-tokens.json` files are imported once and archived as `.bak`. OAuth tokens are encrypted with Electron `safeStorage`.

## Structure

- `app/pages/` contains separate Dashboard, Online, Computer, History, Friends, and Settings routes (Settings shows one category at a time, chosen from the sidebar).
- `app/stores/` holds the Pinia stores that keep game and account state alive while navigating: `kchess.ts` (games, navigation, settings), `friends.ts` (followed players), and `usage.ts` (data downloaded and stored). `app/components/` holds reusable board, game row, and page header components.
- `app/assets/` and `app/utils/` hold styles, media, and chess utilities.
- `src/main/` and `src/preload/` contain Electron's desktop integration.
- `src/shared/` contains types and pure helpers shared by both sides, including `validate.ts`, which checks every value the renderer sends over IPC.
- `nuxt.config.ts` enables client rendering and hash routing, so the generated app works from Electron's local file URL without a running server. Nuxt's generated files live in `.output/public/` and are included in the macOS package.

Nuxt UI is loaded as a Nuxt module. Desktop APIs remain exposed through Electron's preload bridge at `window.kchess`.

## Security notes

- The renderer is sandboxed with context isolation; the main process validates all IPC input and never lets the renderer choose which executable to run: an engine path must come from the native file picker or be the copy KChess downloaded (the bundled engine needs no path). Deleting only ever removes KChess's own download folder.
- The Lichess OAuth loopback server only accepts the `/callback` request that carries the expected `state`.
- The window cannot navigate away from the app's own pages.

## License

KChess is free software, released under the **GNU General Public License v3.0 or later** (see [`LICENSE`](LICENSE)). You may use, study, change, and share it under those terms; anything you distribute that is based on it must stay under the GPL and come with its source.

This is required, not just chosen: the app builds on copyleft code and assets from the Lichess and Stockfish projects.

- `@lichess-org/chessground` and `chessops` — GPL-3.0-or-later
- `stockfish` (the bundled WASM engine) — GPL-3.0
- `@lichess-org/types` (type definitions only, nothing of it ships at runtime) — AGPL-3.0-or-later
- Lichess board themes, sound effects and some piece sets — AGPLv3+ (other piece sets are GPL, MIT, Apache-2.0 or CC0; see the attribution file)
- The rest are permissively licensed (MIT, ISC, Apache-2.0, BlueOak).

The full list, with what each part is used for, is in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md); asset credits are in [`app/assets/ATTRIBUTION.md`](app/assets/ATTRIBUTION.md). The app icon is original artwork drawn by [`scripts/make-icons.py`](scripts/make-icons.py) and covered by the same license.

## Not affiliated

KChess is an independent project. It is **not affiliated with, endorsed by, or sponsored by** [Lichess](https://lichess.org) or the [Stockfish](https://stockfishchess.org) team. It talks to Lichess through Lichess's public API and OAuth, and uses its open-source assets under their licenses. Bulk downloads (game sync, profiles) run one request at a time, as Lichess asks; during a live game it checks connection status about every 3 seconds, and every request is counted in Settings → Data & storage. "Lichess" and "Stockfish" are names of their respective projects.

## Contributing and security

Contributions are welcome; see [`CONTRIBUTING.md`](CONTRIBUTING.md). To report a vulnerability, follow [`SECURITY.md`](SECURITY.md) rather than opening a public issue.
