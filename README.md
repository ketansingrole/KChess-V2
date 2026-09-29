# KChess

KChess is a TypeScript desktop chess app built with Electron, Vue 3, Nuxt UI, and Tailwind CSS. Chess rules and game replay use `chess.js`; the Electron main process handles Stockfish, Lichess, local storage, and OAuth.

## Run

Requires Node.js 22.12+ and npm.

```bash
npm install
npm run dev
```

`npm run build` runs Vue/TypeScript checks and builds the Electron main, preload, and renderer bundles. `npm start` opens the built app.

On macOS, `npm run pack:mac` creates an unsigned local `.app` in `dist/mac-arm64/` (or the matching architecture directory).

## Features

- Dashboard with public Lichess profile ratings, rating history when Lichess provides it, and recent games.
- Computer games at low, medium, and high Stockfish strengths, with move history, replay, takeback, board flip, and annotations.
- Live Lichess play via OAuth, public seeks, direct challenges, clocks, resign, and takeback offers.
- Synced Lichess history with account, result, rated, and page-size filters, plus board replay and auto-play.
- Board themes, appearance, sound, account management, and managed Stockfish installation on macOS Apple Silicon.
- `Cmd+F` for navigation search and `Cmd+,` for Settings.

On first launch on macOS, the app imports accounts, game history, settings, and available tokens from a previous Rust KChess installation at `~/.kchess/kchess.db`, when present. The old database is left untouched. New app data is stored in Electron's user-data directory; OAuth tokens are encrypted with Electron `safeStorage`.

Nuxt UI is installed through its standalone Vue/Vite integration. This app does not run a Nuxt server.
