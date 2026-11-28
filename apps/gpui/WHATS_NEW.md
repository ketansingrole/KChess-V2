# What's new in KChess

The app shows these notes before an update installs and once after it, and keeps them under
Settings → Updates → What's new; the GitHub release and the App Store use them too. They are for
players, not developers: see "What's new" in AGENTS.md. Under Unreleased below, keep the summary
up to date and add one plain sentence (280 characters at most) under Added, Changed or Fixed;
`cargo xtask release-version` turns it into the new version and its date, and refuses to release
with nothing there.

## Unreleased

The board editor sets up well-known openings and endgames in one click, and Guess the move has 22 famous games and takes games from your databases. Settings → Storage shows what KChess keeps and helps you free up space, KChess stays connected to Lichess when left open, the board plays as Lichess's does, and many pages are cleaner.

### Added

- Set the board in the board editor sets up the starting position, an empty board, a popular opening or a classic endgame such as the Lucena or Philidor position.
- Guess the move lists 22 famous games by era, from McDonnell against La Bourdonnais in 1834 to Kasparov's Immortal in 1999.
- Guess the move can take a game from your databases. Search for a player and you guess the moves as them.
- Clean up in Settings → Storage removes the files earlier versions of KChess left behind, which nothing uses any more.
- Drag the corner of the board to make it smaller, and every board page keeps that size. Double-click the corner to make the board as large as fits again.
- Settings can turn off the highlight of the last move and of a king in check, as on Lichess.
- Several premoves in a row, in Settings → Gameplay, lets you line up premoves that play one after another as each turn comes. It is off unless you turn it on, so a new premove replaces the last one as on Lichess.

### Changed

- The choices beside the board in Practice and Puzzles each fit on one row. Their switches, and those in the engine settings, sit at the end of the row, as in Settings.
- While the engine works out its other lines, their rows show placeholders that the lines fill as they arrive. Pointing at a move in a line shows its position at once, under the lines, and the board follows you from move to move.
- On the Database page, Import PGN comes first and the downloads share one Add games menu. With no databases yet, the page shows where to drop a PGN file.
- On the Watch page, a TV game shows the players' head-to-head as a table: this match and all time, the leader in green.
- Moves are listed the same way on every page: the Analysis board and Books show numbered rows like the game pages, with variations and comments under the move they branch from. Move lists are more compact, with every other row shaded.
- Searching for a player on Players, Following, Guess the move and the database's player download lists matching Lichess players under the field as you type; pick one with the arrows or the pointer. Players has a way back to the leaderboard.
- On the Books page, your books end with a card to open another, an open book has a way back to them, and its page buttons float at the foot of its pages. The diagram buttons sit under the board.
- Filters, search fields and buttons that share a row are the same height.
- The tabs on Game history, Over the board, Studies and Openings are only as wide as their names, so they sit together instead of stretching across the page.
- The board editor speaks up only when the position is not legal yet.
- Over the board leaves whose turn it is to the board, and speaks up for a check or a paused clock.
- Insights looks like the rest of KChess: its figures and breakdowns sit in cards, and its filters match the other pages.
- While a page loads, it shows the outline of what's coming, such as Insights' cards or a list of games, instead of a spinner, so nothing jumps when it arrives.
- Insights opens at once when you come back to it with the same filters, and Game history, Studies and Home open faster.
- A game or run with no clock shows ∞ where it said Untimed or No clock.
- Settings → Storage shows what KChess keeps on your computer in a bar and a list, largest first, each in its own colour, with Update or Delete beside what you downloaded. Your disk's free space is shown with what you can still download.
- A mark you give a move on the Analysis board, such as ?? or !?, shows on the board beside the move, ahead of the review's own.
- The board handles as it does on Lichess: your move lands the moment you make it, so you can premove straight away; a clicked move slides while a dragged one drops in place, castling moves the rook at once, and a piece taken fades as its taker arrives.
- The promotion picker opens on the pawn's file, queen first, as on Lichess. Hold Ctrl to choose the piece instead of an automatic queen, even for a premove, and premoves are offered only where the move could still be legal on your turn.

### Fixed

- In a long game the move list keeps the move on the board in view, the newest as it is played, instead of staying at the first moves until you scroll.
- Challenges and new Lichess games keep arriving when KChess has been open for many hours. A quiet connection used to stop for good after a few network drops, until you restarted KChess.
- The analysis board no longer shifts sideways when you turn the engine off.
- Looking up a Lichess player who doesn't exist names them, instead of saying "Lichess has no such thing."
- Many smaller fixes.

## 2026.11.10 (2026-11-27)

KChess is now a chess database too. Keep millions of games on your computer, from your own files, top events, engines, strong Lichess players or the opponent you are preparing for, and study them with no connection.

### Added

- The Books page opens a chess book's PDF beside a board. Click a diagram to set it up, work out the moves yourself, and turn Stockfish on when you want it. Scroll through the pages, and pick up any book you have opened at the page and diagram where you left it.
- The Database page lists the games in your databases. Search them by player, event, year, rating or result, open any one on the analysis board, or play through it in Guess the move, guessing the winner's moves.
- You can import a PGN file, or a .zip, .7z or .bz2 of them, however large: drop it on the Database page.
- The Database page downloads the games of top events Lichess has broadcast since 2020, every game of the Top Chess Engine Championship, or a month of games between strong Lichess players, and brings in the new ones when you update.
- Player's games downloads anyone's Lichess or Chess.com games to prepare against them: see how they play each colour, pick them in Insights to see where they go wrong and how they use their clock, or as the Opponent in Play with computer to practise against their openings.
- Your Lichess and Chess.com games, and the games you finish here, make a Your games database that keeps itself up to date.
- The explorer's Database tab shows the moves played from any position in your databases, offline.
- More → Games with this position, on the analysis board, lists every game that reached it.
- More → Opening report shows how games went from the position, its main lines and its strongest games.
- The analysis board marks N on the first move of your game that no game in your databases played, and shows what was played there instead.
- A game's menu → About opens a report on either player: how they score with each colour, their openings, their rating over time and their best wins.
- A game's menu → Event table shows the tournament it was played in: who finished where, with tiebreaks and performance ratings, and for up to 16 players each one's result against every other. Click a result to open that game.
- Practice → Repertoire → Check lists moves worth a look: in your databases, moves of yours few chose or that scored poorly; in your games, where you left your lines; in a player's downloaded games, what they play against it, unanswered first. One click adds the move you need.
- Practice → Repertoire → From games makes a repertoire out of your games, or a player's you downloaded: the move played most each time and the common replies, ready to drill.
- More filters on the Database page finds games by length and time control, by an ending they reach, like rook against bishop (R vs B), by a pawn structure, like an isolated queen pawn, or by pieces kept on squares, like a knight on d5 against a pawn on e6 (Nd5 vs e6).
- You can copy games into another database, save them as a PGN file or delete them, one at a time or every game a search finds, merge the games a search finds into one study with every line kept, and save a game from the analysis board into any database.
- Tidy up removes games that came in twice and merges the spellings of one player's name, like "Carlsen, M" and "Carlsen, Magnus".
- Practice → Endgames → Your endgames finds where a move in your games threw away a won or drawn endgame, checked with the endgame tablebases, and lets you replay it until you get it right. Insights shows how often you win won endgames and hold drawn ones, like KR vs K.
- Insights shows how you score with each pawn structure you had or played against, like an isolated queen pawn or the Carlsbad. Click one to see those games on the Database page.
- Practice → Endgames → Missed endgames brings back the endgame drills you failed, your own endgames included: the next day, then a week later, then further apart each time you get them right.
- Today's practice on Home lists what is due: repertoire lines, mistakes from your games and missed endgames. Click one to practise it.
- Practice → Visualise trains you to see moves in your head: read the next few moves of a famous game without them being played, then say what is where. The moves then play out on the board, and the levels read further ahead.
- The endgame tablebases download in Settings → Storage, so the analysis board shows how any position with up to five pieces ends, with no connection. A game review marks where an endgame you had won or drawn slipped away, and the move that kept it.
- Play with computer can play like people at your level: pick Players rated 1600 as the Opponent, and it opens with the moves they play and makes the kind of mistakes they make, not random blunders.
- Paste or drop a picture of a chess board, like a screenshot from a website or a book, and KChess sets up that position in the board editor for you to check. If the picture was drawn from Black's side, one click turns it around.

### Changed

- A game you start on any connected Lichess account, even on the website or your phone, now turns engine help off until it ends.
- Settings → Storage shows everything KChess takes, the app included, and what your disk has left, with every download in one place. KChess asks first when a download would almost fill your disk.
- What's new opens with a summary and lists each release's changes as Added, Changed and Fixed. Before an update installs, it shows what the update brings, including updates you skipped, and only once. You can turn it off in the same window.

### Fixed

- Closing the window on Windows and Linux keeps KChess in the tray when you chose Keep running.
- Updates download on their own again after an offline start.
- Settings → Updates shows “Update available” again.
- Voice input no longer freezes KChess.
- Many smaller fixes.

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
