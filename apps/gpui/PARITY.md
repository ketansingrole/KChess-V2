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
- [x] Window size, position and mode remembered in the profile's `window-state.json` (shared with
      Electron), back on a display that can show it
- [ ] Interface zoom (see Settings)

## Shell

- [x] Named KChess in the menu bar, Dock and app switcher (the executable is `KChess`, so a
      `cargo run` build is too; the preview package is "KChess Preview" on purpose)
- [x] Dock, app switcher and Mission Control icon: the light or dark tile by the system
      appearance, set at runtime as Electron does (`src/dock_icon.rs`), so a development build
      has it too
- [x] Traffic lights where Electron puts them (12pt in, beside the toggle) at every interface size
- [x] Sidebar with the same pages, icons and order; collapses to icons
- [x] Sidebar frame as `USidebar`: 256px, a 64px rail collapsed, easing between them in 200ms;
      lines under the header row and over the footer; the edge below the header row on macOS
- [x] Sidebar toggle fixed at x84 beside the traffic lights (x12 elsewhere) open or collapsed,
      28px with a 16px icon, Hide/Show sidebar as tooltip and name, the top bar making room for
      it beside the rail; open or collapsed is kept for both apps (`kchess:sidebar-open`)
- [x] Theme colors, light and dark, following Settings or the system
- [x] Menus: KChess (Services, Hide, Quit), File, Edit, View, Window
- [x] Search box and command palette in the top bar: ⌘K (View → Search…), ⇧⌘P for commands,
      `@` players, `#` settings, `?` help; ranked fuzzy matches highlighted, recent entries, the
      game to return to; ⌘, opens Settings. Drawn as Electron's: one bordered box up to 600px
      wide and 32px tall, centred in the top bar, that grows down into the results and a footer
      of key chips; ⌘K chips hidden below 560px
- [x] Connect Lichess and the account menu in the sidebar footer (play as, connect another,
      Settings, log out); signing out starts the pages afresh
- [x] Sidebar footer as Electron's: Settings, then Connect Lichess (off while offline or busy) or
      the account button with its up-down chevron, 32px rows 4px apart, left-aligned, icons alone
      with tooltips on the rail; the account menu above the button and as wide; hover and press
      colors of Electron's neutral ghost buttons
- [x] Sidebar item spacing (32px rows touching, 20px icons 6px from the label, as Electron)
- [x] Update button in the top bar
- [x] Offline badge (a route out, as `navigator.onLine`), "Working…" while Lichess is waited on,
      and the sidebar's dot for a Lichess game (your move, or going on)
- [ ] Startup screen (the GPUI app reads the profile before its window opens)
- [x] macOS keeps KChess running when the window closes; the Dock opens it again
- [x] ⌘W, ⌘M, Zoom and full screen act on the window (they were lost while it was busy)
- [x] Two-finger swipe back and forward through the pages visited, with the edge arrow and the
      page following the fingers (Settings' swipe switches); the schedule keeps its sideways scroll
- [x] Z turns zen mode on and off on the game pages; the arrows step through moves

## Play with Computer

- [x] Board: board and piece looks, coordinates, last move, check, legal moves, captures
- [x] Click and drag to move, promotion picker, typed moves (SAN or UCI)
- [x] Stockfish at the chosen level; waits for online recovery before using the engine
- [x] Clocks with increments, Game options (start position, Chess960, FEN, clock, side, level)
- [x] Move list, navigation, switch sides, take back, resign, new game, confirmations
- [x] Opening name, status banner, material difference, blindfold and zen toggles
- [x] Saved session and Game history, flushed before quit
- [x] Sounds (move, capture, low time) at the volume Settings chooses (rodio, Symphonia MP3)
- [x] Desktop notification when Stockfish moves, under the Notifications settings
- [x] Premoves (Chessground's targets, queued in blue, played when legal on your turn, a queen
      for a premove's promotion when Settings say so) and pieces sliding to their squares at the
      Settings speed
- [x] Review game: the game on the analysis board at the move shown, reviewed
- [x] Export: animated GIF of the game, PNG of the position shown, PGN file
- [x] Voice input: moves, “take back”, “new game”, “resign”, “switch sides” (asked to confirm),
      choices to pick when unsure, hold to speak, and each phrase in the voice history with the
      move it meant
- [x] Right-click arrows and circles on every board (Shift/Ctrl red, Alt/⌘ blue, both yellow),
      drawn again to take off, cleared by a left click

## Over the board

- [x] Shared board: both sides move by click, drag or typing; flip after each move
- [x] New game: variant with its hint, Chess960 number, FEN, clock, Black's odds minutes
- [x] Clocks start on the first move; pause and resume, take back, draw, resign, Copy PGN
- [x] Chess clock: two faces, side by side or stacked, Start/Pause/Reset, minutes and
      increment, Space presses the running face, P pauses
- [x] Rotating each clock face a quarter turn at a time (`kchess:clock-rotation`); a turned face
      is drawn as a picture of its text, since GPUI cannot rotate text
- [x] Export (GIF, PNG, PGN) and Analyse
- [x] Voice input for either player
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
- [x] Notifications' "Try it" (a sample after 5 seconds, and the system's notification settings)
- [x] Interface size: Auto (by display) and 50–300%, ⌘+ ⌘− ⌘0 and the View menu, shared with
      Electron (`interface-zoom.json`); a window zoom patched into GPUI (`vendor/gpui-pre`,
      `KCHESS-PATCH.md`) scales everything as Electron's page zoom does. Board pages stack the
      panel under the board on narrow pages, as Electron's do
- [x] Voice input (privacy settings, a microphone level test, confirm moves, hold to speak,
      keep history) and the voice history (weak words, heard → meant, recent phrases, export,
      clear); the offline voice model's card in Offline downloads
- [x] Microphone access status and Allow microphone (AVFoundation on macOS, one of the app's
      two `unsafe` exceptions, in `src/microphone.rs`; the Dock icon is the other), read again on coming back from System Settings
- [x] Data & storage: downloads and storage per account (reset counters), Clear local scores,
      Export diagnostics (the redacted, bounded `logs/kchess.log` both apps write)

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
- [x] Voice input: moves played at once, “back”, “next”, “start”, “end”, “best move”, “flip
      board”, “engine”
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
- [x] Voice input on every puzzle board (Train, Daily, runs, mistakes)
- [ ] Theme groups in the picker, Connect or Reconnect from the panel, choosing the rated
      account

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
- [x] Voice answers: Say the square, light or dark, the knight's next square (a run waits for
      the model and microphone before its clock starts)

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
- [x] Voice input: “white knight F three”, “remove E four”, “clear board”, “black to move”,
      “flip”, “analyse”

## Following

- [x] The players followed: ratings, online or playing (presence every 20 seconds), the record
      against them, their synced games; Follow player, Import from Lichess, Sync all, and each
      player's Games, Sync, Clear data and Remove
- [x] The player link, Watch and Challenge

## Players

- [x] Look up a player (recent lookups and followed players offered); profile, online or
      playing, your record against them, rating history (rebuilt from rated games when Lichess
      sends none, with Retry), their record in each speed they play (results, streaks, best wins
      and worst losses, each a player link), recent games opened on the analysis board
- [x] Follow player, Message (with Reconnect when the login lacks the permission)
- [ ] Rating history as one chart of every speed with toggles (one speed at a time here, as on
      Home)
- [x] Watch and Challenge

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

## Tournaments

- [x] The arena schedule as a timeline (rows by series, coloured by kind, opened on now), Playable
      here or All, Refresh
- [x] A tournament: details, conditions, Join (with an entry code), You are in, rank, Pause or
      Withdraw, standings by page with the podium, the top game and games in progress (watched),
      totals; refreshed every 15 seconds while it runs
- [x] Create arena, and Connect when no account is connected
- [x] Go to your game
- [ ] A masked entry code field (the kit has no masked input)

## Lichess insights

- [x] An account's synced games: games, score, streaks and accuracy; by colour, speed, opponent
      rating, length, day, time of day and ending; openings with the best and weakest; filtered by
      account, speed, period and rated or casual; read again when shown

## Play on Lichess

- [x] The live game app-wide: the Rust rules' state machine (`onlineGame*`) driven as
      `OnlineGame` drives it, its clock, sounds, the low-time alert, and a pairing taking you to
      the board; resumed after a restart
- [x] Find a game: account, real time or correspondence, variant, time control, increment, colour,
      rated, a direct challenge (unlocking Blitz), what Lichess blocks, the summary and data used
- [x] Seeking with Cancel; challenges in and out (accept, decline with a reason, withdraw) and the
      lobby's status; games in progress with their boards, Play/Open, polled for "your move"
- [x] The game: players with ratings, clocks and connections (ping, online, away), the moves and
      reviewing them, first-move deadline, draw and takeback offers, the opponent leaving (claim the
      win or a draw), berserk, takeback, offer draw, resign (asked first), abort, close a
      correspondence game; after it: rematch (and the opponent's offer), Analyse, Export,
      Tournament, New game; Reconnect while the connection is down
- [x] The chat dock: the opponent's room and spectators', Lichess's quick messages, unread count
- [x] Desktop notifications with Electron's Settings rules (in-app while KChess is in front)
- [x] Premoves on this board (not in correspondence)
- [ ] Blindfold play on this board
- [ ] The connection panel's ping graph (the line shows the latest round trip)

## Pages not started

None.

## Packaging

- [x] Packages for macOS (.dmg, .zip), Windows (.zip) and Linux (.tar.gz) with the official
      native Stockfish and the voice runtime (helper, libvosk, notices) beside the app (`tooling/gpui-package.mjs`; the preview installs beside
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
