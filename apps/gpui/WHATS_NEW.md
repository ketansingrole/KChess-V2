# What's new in KChess

The app shows these notes once after an update, and keeps them under Settings → Updates →
What's new; the GitHub release and the App Store use them too. They are for players, not
developers: see "What's new" in AGENTS.md. Add a short line (280 characters at most) under
Unreleased below; `cargo xtask release-version` turns it into the new version and its date, and
refuses to release with nothing there.

## Unreleased

- **Your own game database**: import a PGN file, or a .zip, .7z or .bz2 of them, as large as you like, then pick Database in the explorer to see the moves played from any position, offline. Narrow it to a player, their colour, a rating or a year to prepare for an opponent.
- **Your games in the database**: your Lichess and Chess.com games and the games you finish here make a Your games database that keeps itself up to date. Pick it in the explorer's Database tab to see how your own games went from any position.
- **Fair play on every account**: a game you start on any connected Lichess account, on the website or your phone, now turns engine help off until it ends, as a game on the account you play from does.
- **Database page**: browse the games in your game databases, search them by player, event, year, rating or result, and open any one. Drop a PGN file on the page to import it. On the analysis board, More → Games with this position lists every game that reached it.
- **Master games**: on the Database page, download over a million games from top over-the-board events since 2020, then add each new month's games with Update master games.
- **Engine games**: on the Database page, download every game of the Top Chess Engine Championship's competitions, the strongest chess ever played, and see which openings the best engines choose. Update engine games brings in each new season.
- **Strong Lichess games**: on the Database page, Lichess games downloads a month of Lichess's rated games and keeps only those between two strong players, 2400 and up or a level you pick, without bullet or bots. Add a month brings in more.
- **Prepare against a player**: on the Database page, Player's games downloads anyone's Lichess or Chess.com games from the months, speeds and rated games you pick, suggesting Lichess names as you type. See how they play as White and as Black; Update games brings in their latest.
- **See what your databases take**: Manage on the Database page, or Settings → Data & storage, lists each game database with its games, size and last update, and their total. KChess asks first before a download when your disk is almost full.
- **Fixes**: closing the window on Windows and Linux keeps KChess in the tray if you chose Keep running. Updates download on their own again after an offline start. Settings → Updates shows “Update available” again. Voice input no longer freezes KChess. Plus many smaller fixes.

## 2026.11.9 (2026-11-22)

- **Keep KChess running**: close the window and KChess can stay in the menu bar or system tray, still telling you about challenges and your moves. Answer a challenge or open a puzzle from its menu. Choose this, and opening KChess when you log in, in Settings → Notifications.
- **Notifications you missed**: the bell at the top right lights up when something came in while you were away, such as a challenge, your move in a correspondence game or a new version. Open it to go there, or dismiss or clear them.
- **Voice in Endgames and Repertoire**: Voice now sits beside the move box in endgame lessons, drills and the opening trainer. Say your moves, “next”, “retry”, “hint” or “back”, and “show move” or “restart line” in the trainer. Puzzles and Your mistakes take “hint” too.
- **Pages that hold still**: the move list, engine depth, Voice button, Studies tabs, charts and opening explorer stay put as you step through a game, pages open without jumping, and a dialog's error leaves its buttons in place. Following keeps its order as players come online.
- **Steadier Lichess games**: your games say whose move it is, and Offer draw and Takeback stay where they are while an offer waits, so Resign never slides under your pointer. A rated puzzle’s rating change appears without pushing the page down.
- **Type or say your answer in every drill**: Coordinates and Knight paths have a box to type the square and Voice under the board, as in your games, and Guess the move takes your guess by voice too. Say the square is now part of Name the square.
- **Matching corners**: buttons, menus, dialogs, cards and panels now round their corners just like the board, as on Lichess. Boards in study and opening cards follow the card's edge, and Settings and the promotion picker stay inside their rounded corners.
- **Roomier broadcasts**: Watch → Broadcasts fills your window with live and upcoming tournaments, their pictures larger on wide screens, finished ones in a compact list below, and a round's boards grow to use the width too.
- **Broadcast alerts**: tap the bell on a broadcast, or pick rounds from its Round alerts menu, and KChess tells you when a round starts, even while it's in the background. Clicking the alert opens the round. Your alerts are listed on Broadcasts, where you can remove them.
- **Fixes**: Watch TV shows its moves at once, and Broadcasts show their pictures again. The opening explorer offers Connect Lichess or Try again. Small buttons are easier to click. Plain-coloured boards look sharper and use less memory. Plus many smaller fixes.

## 2026.11.8 (2026-11-21)

- **Icons the right size everywhere**: icons now keep the same size whatever your screen and its scaling, so none look too small beside the others on Windows, and the button that shows and hides the sidebar lines up with the icons below it.
- **Zen mode on more boards**: Z and the Zen button now hide the sidebar in your Lichess games and the practice trainers too, not just against the computer or over the board. Guess the move is now found from Search.
- **Brilliant moves**: game reviews run on your computer now mark your brilliant moves too.
- **Say “next” in puzzles**: with voice on, voice keeps listening once you finish a puzzle or a mistake drill, and saying “next” takes you to the next one.
- **See a line before you play it**: hover over a move in the engine lines, the Explorer's move list or a ChatGPT explanation to see the position it leads to.
- **Find a setting in one go**: search for a setting at the top of the window and it opens Settings right at that row, lit up. Searching History, Studies and Openings now highlights the words that matched.
- **Captured pieces on the analysis board**: the pieces each side has won, and who is ahead, show above and below the board as they do in your games.
- **Easier on your battery**: on a laptop running on battery, automatic game reviews now wait for a charger unless Also on battery power is on, and the engine uses fewer cores.
- **Engine back after a Lichess hiccup**: if KChess can't reach Lichess when it starts, the engine, game review and Play with computer now say so and come back on their own once Lichess answers, instead of staying off until you restart.
- **A board you can hear**: with a screen reader, each move now says its piece and any check ("Last move knight g1–f3, check"), and the board says the hint or solution it shows, in puzzles, endgames and the opening trainer.
- **Try a setting, then Undo**: change a setting and a notice offers Undo. A setting you changed shows a reset button for its default, and Restore defaults resets a whole Settings section. New players' premoved pawns now become a queen, as on Lichess; your own choice stays.
- **Microphone check on Windows**: Settings › Voice input now shows whether Windows lets KChess use your microphone, and the test tells you when your microphone is silent or too quiet.
- **Move sounds come back**: after you switch speakers or headphones, or a Bluetooth speaker reconnects, the next move plays its sound again. On Windows, Bluetooth speakers that stayed silent, like an Echo Show, now play move sounds too.
- **Sounds on the analysis board**: moves you play on the analysis board now make the move and capture sounds, and so does stepping forward through a game one move at a time.
- **Fixes**: A wrong puzzle move goes back, so you can try again. Cancel seek stays after you sign in to Lichess. Analysis keeps going as you step through moves. Watch shows the moves of rapid games. Double-clicking the top bar zooms the window once. Plus many smaller fixes.

## 2026.11.7 (2026-11-09)

- **Smarter Guess the move**: guess the games of any Lichess player, or a famous game like the Opera Game. When a player went wrong, finding the better move earns full points.
- **Reply from the notification**: when it's your move in a correspondence game, type your move right into the notification on Windows or Mac, and confirm it before it's played.
- **A tidier sidebar**: your account sits at the bottom of the sidebar, with Settings beside it. Its menu also opens What's new and lets you report a problem.
- **Notifications you can click**: Go to board takes you straight to the game, and you can accept or decline a Lichess challenge from its notification. On Windows they no longer show as Windows PowerShell.
- **Settings in a window of its own**: Settings opens over the page you were on, with its sections down the left. Close it with ✕ or Escape to carry on where you were.
- **A calmer Over the board page**: Shared board and Chess clock are tabs at the top, and the chess clock is bigger, with its settings in a list beside it.
- **Clearer dialogs**: dialogs open in the middle of the window and dim the page behind them.
- **Much smaller updates**: an update downloads only what changed, usually a few MB instead of over 100 MB, so it's ready in seconds.
- **Faster with a long game history**: with hundreds of saved games and studies, KChess starts sooner, its pages open faster, and Studies and Game history scroll smoothly.
- **Fixes**: broadcast rounds always show their boards, voice input no longer gets stuck on "Preparing voice input…", Puzzle history no longer shows the previous account's results after you sign out, and a server hiccup no longer stops Stockfish downloading. Plus many smaller fixes.

## 2026.11.5 (2026-11-07)

- **Fixes**: Puzzles opens on Train instead of an empty page when it cannot find the tab you left it on.

## 2026.11.4 (2026-11-07)

- **What's new**: after each update, KChess shows you what changed. You can read it again any time in Settings → Updates.
- **Crazyhouse**: a new way to play, where captured pieces join your side and can be dropped back on the board. Play it against the computer, with a friend, or on Lichess.
- **Guess the move**: play through a famous game and try to guess each move the player made. Find it in Practice.
- **Smarter opening practice**: the opening trainer brings back the moves you got wrong until you remember them.
- **Chess.com games**: bring in your Chess.com games, and see a Chess.com player's recent games on Home.
- **How you use your clock**: Insights now shows where you spend your time in your games.
- **Open many games at once**: drag a file with several games onto KChess to open them all together.
- **Play without a mouse**: move pieces with the keyboard, and screen readers read out every square.
- **More for other chess variants**: set up positions for any variant, practice your mistakes from variant games, and get your Lichess variant games reviewed.
- **A clearer Home**: see who won each game, the result of a finished game, and your rating history at a glance.
- **Calmer animations**: KChess follows your computer's reduce motion setting.
- **Find us**: Settings → Updates links to the KChess website, its source code, and a place to report a problem.
- **More reliable updates**: if an update download is interrupted, it carries on where it stopped.
- **Coming from an older KChess**: your studies, page settings and every saved Lichess login move over the first time you open KChess, even accounts you were not signed in to.
- **Fixes**: KChess uses less memory over long sessions, is safer, and busy pages respond faster. New opponent works right after a Lichess game ends, instead of asking you to reconnect.

## 2026.11.3 (2026-11-04)

- **Clearer error messages**: when something goes wrong with Lichess, KChess says what happened and how to fix it.
- **Watching games**: games you watch start right away, their clocks keep ticking, and they end cleanly when the game is over.
- **Following players**: see what the players you follow have been up to.
- **Small touches**: large text is no longer cut off, and buttons and switches line up neatly.
