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
- [x] Sounds (move, capture, low time) at the volume Settings chooses (rodio, Symphonia MP3)
- [ ] Desktop notification when Stockfish moves in the background
- [ ] Premoves and piece animation
- [x] Review game: the game on the analysis board at the move shown, reviewed
- [ ] Voice input, Export (PGN, GIF)
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

## Settings

- [x] Sidebar lists the categories with Back; changes save after 400ms, in order, and at quit
- [x] Appearance (theme, coordinates, animation, piece set, board), App theme (light and dark
      pickers, own themes folder and reload), Gameplay (levels included), Gestures, Online play,
      Analysis, Sound, Notifications (switches)
- [ ] Interface size; Notifications' "Try it"; Voice input, Chess engine, My accounts, Offline
      downloads, Data & storage, Updates

## Game history

- [x] Computer and Over the board tabs: result filters with counts, the list, 20 to a page,
      a game opened to go through move by move (`replay.rs`), Export PGN, Copy summary, Delete
- [x] One list of played games for every page (`played.rs`), written in order and at quit
- [x] Open analysis
- [ ] Lichess tab (synced games, filters, review); swipe back and forward
- [ ] Export copy inside the delete dialog

## Home

- [x] Start tiles (Continue your game with its move, side and level), shortcuts, recent games
      opening in Game history, Connect Lichess (signs in, then syncs)
- [ ] Your Lichess activity (stats, rating chart, synced games) and Sync games for a connected
      account; Connect Lichess is not yet tried against Lichess from this app

## Analysis board

- [x] The session both apps keep (`sessions.analysis`): the tree, the move shown, orientation
- [x] Move tree with variations and comments, typed moves, keys (← → ↑ ↓, F, L, Space), wheel
- [x] Stockfish lines and arrows, evaluation bar, engine settings, Lichess cloud evaluation, the
      same evaluation cache and paused engine during a live game
- [x] Import FEN or PGN, Copy FEN and PGN, Make main line, Delete from here, New analysis
- [ ] Comments and annotations (`MoveNotes`), the move list's right-click menu, scrolling the
      shown move into view
- [x] The study bar: the open study saved as you go (to its chapter), its name edited in place,
      chapters (switch, add, rename, duplicate, delete), Save as study, Save a copy, Close and
      Delete study with Undo, Game details; a recovered session that differs from its study stays
      an unsaved copy, as in Electron
- [x] Review: the core's game review of the main line (asked for, followed while it runs, or
      found stored), accuracy, labelled moves (next by count), centipawn loss and phases, the
      evaluation chart, labels in the move list and on the board, the engine paused while this
      game is reviewed, your mistakes saved once it completes, and practising the game's mistakes
      on the board
- [ ] Explorer tool, Export, voice input
- [x] Edit this position, Play Stockfish or a friend from here

## Puzzles

- [x] Train: Practice, Downloaded and Rated modes on the Rust puzzle session; theme (from the
      shared `data/puzzle-themes.json`), difficulty and side; hint ring, solution, wrong-move
      flash, next after a solve; the session tally, day streak and attempt strip from the log
- [x] The puzzle database card: download with progress, cancel, delete
- [ ] Daily, Storm · Streak · Rush, Lichess stats and History tabs; the attempt strip's menu
- [x] An opening's puzzles, chosen on Openings
- [ ] Theme groups in the picker, Connect or Reconnect from the panel, choosing the rated
      account, voice input

## Practice

- [x] Coordinates (find and name, lengths, sides, some files and ranks, coordinates on every
      square, pieces), Square colours and Knight paths, with their best scores and averages kept
      by the core (`runSummary`, `saveRun`)
- [x] Repertoire: drill a saved study (one chapter or all) or an opening chosen on Openings
      (its named line, then the explorer's popular moves), as White or Black, steered towards the
      lines missed before (`recordRepertoireMiss`); Show move, Restart line, Forget mistakes and
      Save as study
- [x] Endgames: the 14 courses (read, follow and solve steps, scripted or Stockfish replies, hints,
      take back) and the 9 drills against Stockfish at full strength, each finished chapter or
      drill kept by the core (`runSummary`, `saveRun`)
- [x] Your mistakes: due, learning and learned counts, the positions on the shared puzzle board
      (hint, solution), rescheduled by the core (`answerMistake`), and the games they come from
- [x] A game's Practice button: its positions practised on the analysis board
- [ ] Voice answers

## Openings

- [x] Search (case, accents and apostrophes folded), Recent, Popular cards with Train, and every
      family by first move as an expandable tree, from the shared `openingTree`
- [x] One opening: a board to explore it (drag or type a move), its moves, Train this line
      (Practice → Repertoire), Analyse (asks before replacing unsaved analysis) and Train with
      puzzles (its Lichess puzzle key as the theme); its Wikibooks excerpt when the core has one
- [ ] The popularity chart and the position explorer beside the line

## Studies

- [x] The library from the core (one writer, flushed on quit): search, sort, cards with their
      position, chapters and last edit; open (asking before replacing unsaved analysis), New
      study, Import PGN (chapters by `ChapterName`), Rename, Duplicate, Copy PGN, Drill as
      repertoire, Delete with Undo, Upload to Lichess (changes or a new cloud copy)
- [ ] The Lichess tab (studies on Lichess, downloading them)

## Board editor

- [x] Drag pieces anywhere (off the board removes them), drag spare pieces in, or paint with a
      spare piece or the bin; side to move, castling the pieces allow, en passant, a FEN typed or
      copied; Starting position, Clear, Flip; Analyse, Play Stockfish, Play a friend
- [ ] Voice input

## Pages not started

Play on Lichess, Tournaments, Watch,
Lichess insights, Following, Players.

## Packaging

- [ ] A native Stockfish shipped beside the app (the core runs it without a runtime)
- [ ] Rust updater reading the same GitHub release feed; the switch from Electron through it
- [ ] Signing and notarization on macOS (the Electron build is unsigned today)
- [ ] CI: build and test `kchess-gpui` on macOS, Windows and Linux
- [ ] THIRD_PARTY_NOTICES.md rows for the app's own crates (GPUI Kit, rodio, Symphonia,
      chrono, dirs) once it ships
