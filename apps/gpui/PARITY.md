# GPUI app parity with the Electron app

The GPUI app (`apps/gpui`) replaces the Electron app once every page here matches it and users
can switch without noticing. Compare a page by opening the same profile in both apps:

```sh
pnpm run build
node tooling/with-branded-electron.mjs node tooling/parity-screens.mjs <dir> light <profile>
KCHESS_USER_DATA_DIR=<profile> KCHESS_ENGINE_PATH=<native stockfish> cargo run -p kchess-gpui -- -NSRequiresAquaSystemAppearance YES
```

`KCHESS_ENGINE_PATH` stands in for the Stockfish the packaged app will ship beside itself.

## Shared with Electron (one profile, either app)

- [x] Profile directory, settings (`desktopSettings`), appearance and app themes, custom themes
- [x] Lichess logins: tokens sealed with the profile's credentials (`secrets.rs`); Electron's
      `safeStorage` tokens are resealed the first time Electron uses them
- [x] View preferences (`viewPreferences`); earlier `localStorage` keys move over once
- [x] Unfinished computer and over-the-board games (sessions `computer` and `local`) and
      their Game history entries; chess clock sessions in Game history
- [ ] Interface zoom and window size and position

## Shell

- [x] Sidebar with the same pages, icons and order; collapses to icons
- [x] Theme colors, light and dark, following Settings or the system
- [x] Menus: KChess (Services, Hide, Quit), File, Edit, View, Window
- [ ] Search box and command palette in the top bar (⌘K)
- [ ] Connect Lichess and the account menu in the sidebar footer
- [ ] Sidebar item spacing (32px rows, 20px icons in Electron)
- [ ] Startup screen, offline badge, "Working…" status, update button
- [ ] Reopening the window from the Dock on macOS (closing it quits for now)
- [ ] Two-finger swipe between pages, keyboard shortcuts (arrows, Z for zen)

## Play with Computer

- [x] Board: board and piece looks, coordinates, last move, check, legal moves, captures
- [x] Click and drag to move, promotion picker, typed moves (SAN or UCI)
- [x] Stockfish at the chosen level; waits for online recovery before using the engine
- [x] Clocks with increments, Game options (start position, Chess960, FEN, clock, side, level)
- [x] Move list, navigation, switch sides, take back, resign, new game, confirmations
- [x] Opening name, status banner, material difference, blindfold and zen toggles
- [x] Saved session and Game history, flushed before quit
- [ ] Sounds (move, capture, low time) — `sound.rs` is silent until a GPL-compatible MP3
      decoder is chosen
- [ ] Desktop notification when Stockfish moves in the background
- [ ] Premoves and piece animation
- [ ] Voice input, Export (PGN, GIF), Review game
- [ ] Right-click arrows and circles on the board

## Over the board

- [x] Shared board: both sides move by click, drag or typing; flip after each move
- [x] New game: variant with its hint, Chess960 number, FEN, clock, Black's odds minutes
- [x] Clocks start on the first move; pause and resume, take back, draw, resign, Copy PGN
- [x] Chess clock: two faces, side by side or stacked, Start/Pause/Reset, minutes and
      increment, Space presses the running face, P pauses
- [ ] Rotating each clock face (GPUI cannot rotate text yet)
- [ ] Voice input, Export (PGN, GIF), Analyse
- [ ] Electron's chess clock (`stores/clock.ts`) still keeps its own copy of the rules now in
      `kchess-domain/src/games/otb_clock.rs`

## Pages not started

Home, Puzzles, Practice, Openings, Analysis board, Game history, Studies,
Board editor, Play on Lichess, Tournaments, Watch, Lichess insights, Following, Players, Settings.

## Packaging

- [ ] A native Stockfish shipped beside the app (the core runs it without a runtime)
- [ ] Rust updater reading the same GitHub release feed; the switch from Electron through it
- [ ] Signing and notarization on macOS (the Electron build is unsigned today)
- [ ] CI: build and test `kchess-gpui` on macOS, Windows and Linux
