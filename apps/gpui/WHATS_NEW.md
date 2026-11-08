# What's new in KChess

The app shows these notes once after an update, and keeps them under Settings → Updates →
What's new; the GitHub release and the App Store use them too. They are for players, not
developers: see "What's new" in AGENTS.md. Add a line under Unreleased below;
`cargo xtask release-version` turns it into the new version and its date, and refuses to release
with nothing there.

## Unreleased

- **Smarter Guess the move**: look up any Lichess player and guess one of their games as them, or pick a famous game like the Opera Game or the Game of the Century. The engine now checks the moves that were played too: when a player went wrong, finding the better move earns full points, and you're told what today's engines prefer.
- **Reply from the notification**: when it's your move in a correspondence game, type your move right into the notification on Windows or Mac. KChess asks you to confirm it before it's played.
- **A tidier sidebar**: your account sits at the bottom of the sidebar with Settings beside it. Its menu also opens What's new and lets you report a problem.
- **Notifications you can click**: notifications come from KChess and Go to board takes you straight to the game, and a Lichess challenge can be accepted or declined right from its notification. On Windows they no longer show as Windows PowerShell.
- **Settings in a window of its own**: Settings opens over the page you were on, with its sections down the left. Close it with ✕, Escape or a click outside to carry on where you were. Each setting is named on the left with its switch or choice on the right, so a section reads at a glance.
- **A calmer Over the board page**: Shared board and Chess clock sit in a tab row at the top, so switching between them no longer shuffles the page. The chess clock fills the space the board takes, with bigger times and its settings in a neat list beside it.
- **Clearer dialogs**: dialogs open in the middle of the window and dim the page behind them, so they stand out from the board.
- **Much smaller updates**: an update now downloads only what changed since your version, usually a few MB instead of over 100 MB (about a third of it for the AppImage), so it is ready in seconds. If anything in your installation is missing or changed, KChess downloads the full update instead. Checking for updates also tries again by itself when the connection drops.
- **Faster with a long game history**: KChess starts sooner, and Over the board, Analysis board and Home open faster, when you have hundreds of saved games and studies.
- **Fixes**: a broadcast round you open always shows its boards, instead of now and then staying empty, and the Voice button under the board is no longer cut off in a narrow window. While you wait for an opponent on Lichess, the waiting card sits in the middle of the page, the time on each clock is centred, the computer's difficulty list shows each level's full name and rating, clicking a selected piece again puts it down, and a king in check glows red within its own square, as on Lichess, with a red # when it is checkmated. On Home, "In progress" and other results fit inside their labels in Recent games, the puzzle stat cards line up at one height, and the square colors trainer is no longer stretched across the whole page. KChess uses less memory over long sessions, and spoken moves no longer get out of step and act on what you said before. Voice input no longer stays on "Preparing voice input…" when the connection drops while its model downloads, a move made in the pause after an opening line ends no longer counts as a mistake in the opening trainer, Puzzle history and Insights no longer show the previous account's results after you sign out, a game review that failed once runs when you open the game again, and reviews start again as soon as a Lichess game ends. Retry install after an update that failed to install installs it again instead of downloading it a second time, and closing a study on the analysis board no longer clears a game you opened while it saved.

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
