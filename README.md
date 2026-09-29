# KChess

KChess is a TypeScript desktop chess app built with Electron, Nuxt 4, Nuxt UI, and Tailwind CSS. Chess rules use [`chessops`](https://github.com/niklasf/chessops), the board is [`@lichess-org/chessground`](https://github.com/lichess-org/chessground), game review is a custom PGN replay built on `chessops` and the same board, and the Lichess API client is typed with Lichess's official [`@lichess-org/types`](https://github.com/lichess-org/api) OpenAPI types through `openapi-fetch`. Sounds and board themes are the Lichess assets. The rating chart uses [Unovis](https://unovis.dev), and IPC input is validated with [Valibot](https://valibot.dev). The Electron main process handles Stockfish, Lichess, local storage, and OAuth.

## Run

Requires Node.js 22.12+ and npm.

```bash
npm install
npm run dev
```

`npm run dev` runs Nuxt and Electron together. `npm run build` type-checks both sides, generates the static Nuxt renderer, and builds Electron's main and preload bundles. `npm start` opens the built app. `npm test` runs the chess-logic and main-process smoke tests (`test:chess` runs just the chess part). `npm run format:check` and `npm run typecheck` run in CI (`.github/workflows/ci.yml`).

On macOS, dev and local runs launch through `scripts/with-branded-electron.mjs`, which keeps a `KChess`-branded copy of the Electron binary in `.dev/` (gitignored) so the menu bar, Dock, and Cmd-Tab show the app name instead of "Electron".

On macOS, `npm run pack:mac` creates an unsigned local `.app` in `dist/mac-arm64/` (or the matching architecture directory).

## Features

- Dashboard with public Lichess profile ratings, rating history when Lichess provides it, and recent games.
- Computer games at low, medium, and high Stockfish strengths, with move history, replay, takeback, board flip, promotion picker, and draw detection (repetition and the 50-move rule).
- Live Lichess play via OAuth, public seeks, direct challenges, clocks, resign, and takeback offers. Connect as many of your own Lichess accounts as you like and choose which one plays (from the sidebar menu or the Play Online page); each has its own encrypted token.
- Synced Lichess history reviewed in a built-in PGN replay (move list, keyboard navigation).
- Lichess board themes, board coordinates (inside / outside / hidden), Lichess move sounds, appearance settings, account management, and a choice of Stockfish in Settings: the bundled Stockfish 19 (the [`stockfish`](https://www.npmjs.com/package/stockfish) npm package's lite multi-threaded WASM build, run as a UCI process by Electron's Node runtime, so it works offline on every platform), a native build you can download (macOS; verified against the SHA-256 digest GitHub publishes for the release) and delete again from Settings, or any executable you pick yourself.
- `Cmd+K` (or `Ctrl+K`) for the command palette (page navigation and actions) and `Cmd+,` for Settings.

On first launch on macOS, the app imports accounts, game history, settings, and available tokens from a previous Rust KChess installation at `~/.kchess/kchess.db`, when present. The old database is left untouched. New app data is stored in SQLite (`kchess.db` in Electron's user-data directory, via Node's built-in `node:sqlite`; schema changes are ordered migrations in `src/main/migrations.ts`, tracked with `PRAGMA user_version`); existing `kchess-data.json` / `lichess-tokens.json` files are imported once and archived as `.bak`. OAuth tokens are encrypted with Electron `safeStorage`.

## Structure

- `app/pages/` contains separate Dashboard, Online, Computer, History, and Settings routes.
- `app/stores/kchess.ts` is the Pinia store that keeps game and account state alive while navigating (`app/composables/useKChessState.ts` is a thin accessor over it). `app/components/` holds reusable board, game row, and page header components.
- `app/assets/` and `app/utils/` hold styles, media, and chess utilities.
- `src/main/` and `src/preload/` contain Electron's desktop integration.
- `src/shared/` contains types and pure helpers shared by both sides, including `validate.ts`, which checks every value the renderer sends over IPC.
- `nuxt.config.ts` enables client rendering and hash routing, so the generated app works from Electron's local file URL without a running server. Nuxt's generated files live in `.output/public/` and are included in the macOS package.

Nuxt UI is loaded as a Nuxt module. Desktop APIs remain exposed through Electron's preload bridge at `window.kchess`.

## Security notes

- The renderer is sandboxed with context isolation; the main process validates all IPC input and never lets the renderer choose which executable to run: an engine path must come from the native file picker or be the copy KChess downloaded (the bundled engine needs no path). Deleting only ever removes KChess's own download folder.
- The Lichess OAuth loopback server only accepts the `/callback` request that carries the expected `state`.
- The window cannot navigate away from the app's own pages.

## Licensing

The app reuses Lichess open source code and assets, which are copyleft:

- `@lichess-org/chessground`, `@lichess-org/pgn-viewer`, and `chessops` — GPL-3.0-or-later
- `@lichess-org/types` — AGPL-3.0-or-later
- `stockfish` (the bundled WASM engine) — GPL-3.0
- Lichess board themes and sound effects — AGPLv3+ (see `app/assets/ATTRIBUTION.md`)

When distributed, the combined work must be released under a GPL/AGPL-compatible license.
