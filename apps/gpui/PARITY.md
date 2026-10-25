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
- [x] Connect Lichess and the account menu in the sidebar footer (play as, connect another,
      Settings, log out); signing out starts the pages afresh
- [ ] Sidebar item spacing (32px rows, 20px icons in Electron)
- [x] Update button in the top bar
- [ ] Startup screen, offline badge, "Working…" status
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
- [x] Export: animated GIF of the game, PNG of the position shown, PGN file
- [ ] Voice input
- [ ] Right-click arrows and circles on the board

## Over the board

- [x] Shared board: both sides move by click, drag or typing; flip after each move
- [x] New game: variant with its hint, Chess960 number, FEN, clock, Black's odds minutes
- [x] Clocks start on the first move; pause and resume, take back, draw, resign, Copy PGN
- [x] Chess clock: two faces, side by side or stacked, Start/Pause/Reset, minutes and
      increment, Space presses the running face, P pauses
- [ ] Rotating each clock face (GPUI cannot rotate text yet)
- [x] Export (GIF, PNG, PGN) and Analyse
- [ ] Voice input
- [ ] Electron's chess clock (`stores/clock.ts`) still keeps its own copy of the rules now in
      `kchess-domain/src/games/otb_clock.rs`

## Settings

- [x] Sidebar lists the categories with Back; changes save after 400ms, in order, and at quit
- [x] Appearance (theme, coordinates, animation, piece set, board), App theme (light and dark
      pickers, own themes folder and reload), Gameplay (levels included), Gestures, Online play,
      Analysis, Sound, Notifications (switches)
- [x] Chess engine (bundled, downloaded or your own Stockfish; download, update, delete) and
      Game review; Offline downloads (the engine, the shared puzzle database card, saved games)
- [x] My accounts: connect, sync, use for online play, disconnect, with what each keeps
- [ ] Interface size; Notifications' "Try it"; Voice input, Offline voice model
      downloads, Data & storage, Updates

## Game history

- [x] Computer and Over the board tabs: result filters with counts, the list, 20 to a page,
      a game opened to go through move by move (`replay.rs`), Export PGN, Copy summary, Delete
- [x] One list of played games for every page (`played.rs`), written in order and at quit
- [x] Open analysis
- [x] Lichess tab: synced games a page at a time, filtered by account, result and rating, with
      their reviews; a game gone through move by move, or reviewed on the analysis board
- [ ] Swipe back and forward
- [ ] Export copy inside the delete dialog

## Home

- [x] Start tiles (Continue your game with its move, side and level), shortcuts, recent games
      opening in Game history, Connect Lichess (signs in, then syncs)
- [x] Your Lichess activity: games and ratings, the rating history by mode and range (built
      from synced games when Lichess has none), recent synced games, and Sync games

## Analysis board

- [x] The session both apps keep (`sessions.analysis`): the tree, the move shown, orientation
- [x] Move tree with variations and comments, typed moves, keys (← → ↑ ↓, F, L, Space), wheel
- [x] Stockfish lines and arrows, evaluation bar, engine settings, Lichess cloud evaluation, the
      same evaluation cache and paused engine during a live game
- [x] Import FEN or PGN, Copy FEN and PGN, Make main line, Delete from here, New analysis
- [x] Comments and annotations (`MoveNotes`, C), the move list's right-click menu (comment,
      Annotate, Make main line, Delete from here, Copy FEN; Copy PGN), scrolling the shown move
      into view
- [x] Export: animated GIF of the main line, PNG of the position shown, PGN file
- [x] The study bar: the open study saved as you go (to its chapter), its name edited in place,
      chapters (switch, add, rename, duplicate, delete), Save as study, Save a copy, Close and
      Delete study with Undo, Game details; a recovered session that differs from its study stays
      an unsaved copy, as in Electron
- [x] Review: the core's game review of the main line (asked for, followed while it runs, or
      found stored), accuracy, labelled moves (next by count), centipawn loss and phases, the
      evaluation chart, labels in the move list and on the board, the engine paused while this
      game is reviewed, your mistakes saved once it completes, and practising the game's mistakes
      on the board
- [x] Explorer tool: Lichess, Masters, Player and Tablebase lookups with their filters, Follow
      moves, saved lookups, playing a move and opening a listed game
- [ ] Voice input
- [x] Edit this position, Play Stockfish or a friend from here

## Puzzles

- [x] Train: Practice, Downloaded and Rated modes on the Rust puzzle session; theme (from the
      shared `data/puzzle-themes.json`), difficulty and side; hint ring, solution, wrong-move
      flash, next after a solve; the session tally, day streak and attempt strip from the log
- [x] The puzzle database card: download with progress, cancel, delete
- [x] Daily: the puzzle of the day on its own board
- [x] History: every attempt kept here and every one Lichess recorded, filtered by mode, with
      today's and all-time totals; Try again, or study it on the analysis board (the solution
      as the line, each move tried a "Your move" variation, reviewed)
- [x] The attempt strip's menu (Try again, Analysis board)
- [x] Storm · Streak · Rush: Storm, Streak and the Rush runs over the downloaded puzzles, with
      the clock, combo, strikes and skips, scores kept here, and Lichess's Storm and Streak
      results beside the menu
- [x] Lichess stats: puzzle rating and games, the dashboard's first-try share and performance by
      theme over a period (Train a theme from it), the puzzle rating history
- [ ] Labels on a studied puzzle's tried moves once the engine has judged them
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
- [x] The popularity chart and the position explorer beside the line (a grid of boards)

## Studies

- [x] The library from the core (one writer, flushed on quit): search, sort, cards with their
      position, chapters and last edit; open (asking before replacing unsaved analysis), New
      study, Import PGN (chapters by `ChapterName`), Rename, Duplicate, Copy PGN, Drill as
      repertoire, Delete with Undo, Upload to Lichess (changes or a new cloud copy)
- [x] The Lichess tab: an account's studies, opened (kept offline first) or kept offline, and
      the analysis board sent to one of them or to a new private study

## Board editor

- [x] Drag pieces anywhere (off the board removes them), drag spare pieces in, or paint with a
      spare piece or the bin; side to move, castling the pieces allow, en passant, a FEN typed or
      copied; Starting position, Clear, Flip; Analyse, Play Stockfish, Play a friend
- [ ] Voice input

## Following

- [x] The players followed: ratings, online or playing (presence every 20 seconds), the record
      against them, their synced games; Follow player, Import from Lichess, Sync all, and each
      player's Games, Sync, Clear data and Remove
- [x] The player link and Watch
- [ ] Challenge once Play on Lichess is drawn

## Players

- [x] Look up a player (recent lookups and followed players offered); profile, online or
      playing, your record against them, rating history (rebuilt from rated games when Lichess
      sends none, with Retry), their record in each speed they play (results, streaks, best wins
      and worst losses, each a player link), recent games opened on the analysis board
- [x] Follow player, Message (with Reconnect when the login lacks the permission)
- [ ] Rating history as one chart of every speed with toggles (one speed at a time here, as on
      Home)
- [x] Watch
- [ ] Challenge once Play on Lichess is drawn

## Watch

- [x] TV: channels (the last one reopened), the live board with ticking clocks, the moves once
      Lichess's export lines up (browse them, Back to live), head-to-head, the channel's earlier
      games opened on the analysis board, Analyse and Stop; the stream stops when the page is left
- [x] Following: followed players playing now, and Watch (also from Players and Following)
- [x] Broadcasts: live, coming up and finished tournaments, search, a tournament's rounds and
      boards with clocks, one board followed move by move (←/→, F), All boards and Analyse
- [ ] Broadcast engine analysis (the board's engine panel and the grid's evaluation bars)
- [ ] Broadcast cover images
- [ ] Swipe back through a broadcast's game and tournament

## Pages not started

Play on Lichess, Tournaments, Lichess insights.

## Packaging

- [x] Packages for macOS (.dmg, .zip), Windows (.zip) and Linux (.tar.gz) with the official
      native Stockfish beside the app (`tooling/gpui-package.mjs`; the preview installs beside
      Electron as "KChess Preview", `--final` takes Electron's identity)
- [x] Rust updater on the same GitHub release feed: check, background download checked against
      `SHA256SUMS.txt`, restart to install or install on quit, the top bar's button and Settings
- [ ] The switch from Electron: the Release workflow publishing these packages, and Electron's
      updater installing the final GPUI build
- [x] Signing: ad hoc without credentials; a Developer ID (`KCHESS_SIGN_IDENTITY`) signs with the
      hardened runtime and notarizes (untried here: no certificate)
- [x] CI: lint, test and package `kchess-gpui` on macOS, Windows and Linux (`gpui` job); Linux
      builds checked in CI's image (`pnpm run linux`)
- [x] THIRD_PARTY_NOTICES.md rows for the app's own crates and its Stockfish
