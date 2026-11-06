# KChess and Lichess: feature parity

This page lists the Lichess features KChess added to close the gap with lichess.org, where each
one lives and which setting controls it. It also lists the features that cannot be built, and
why. KChess talks to Lichess only through its public API and OAuth (the Board API for play), so
whatever that API does not offer cannot be built in the app.

## Implemented

### Live play (Play Online)

| Feature                     | How it works                                                                                                                                                                                                                                             | Settings                                                                    |
| --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| Draw offers                 | Offer, accept, decline or withdraw a draw (`/api/board/game/{id}/draw/{yes\|no}`). Opponents' offers show as a banner.                                                                                                                                   | —                                                                           |
| Takeback requests           | The opponent's request now shows as a banner with Accept / Decline (sending your own request already existed).                                                                                                                                           | —                                                                           |
| Claim victory / call a draw | When the opponent leaves, a countdown runs until Lichess allows a claim, then _Claim victory_ and _Call it a draw_ appear (`claim-victory`, `claim-draw`).                                                                                               | —                                                                           |
| First-move countdown        | Shows how long you have to make your first move before Lichess aborts the game.                                                                                                                                                                          | —                                                                           |
| Chat                        | Player chat with Lichess's quick messages; the spectator chat can be shown too (`/api/board/game/{id}/chat`).                                                                                                                                            | Settings → Online play → _Game chat_                                        |
| Incoming challenges         | An event stream for the active account stays open while you are not playing. Challenges and rematch offers appear on Play Online and as notifications; accept them, or decline with one of Lichess's reasons. You can also withdraw your own challenges. | Settings → Online play → _Receive challenges_; Notifications → _Challenges_ |
| Rematch                     | After a game, _Rematch_ sends the same opponent a challenge with the same settings and colours swapped. _New opponent_ seeks the same clock and variant against anyone. An incoming rematch offer (`rematchOf`) shows as a banner.                                                                                       | —                                                                           |
| Berserk                     | Shown in arena games before your first move.                                                                                                                                                                                                             | —                                                                           |
| Lifetime score              | Your lifetime score against the opponent (crosstable) shows beside the board.                                                                                                                                                                            | —                                                                           |
| Variants                    | Seeks and challenges in Chess960, King of the Hill, Three-check, Antichess, Atomic, Horde, Racing Kings and Crazyhouse. Boards replay every variant with chessops rules, and Chess960 castles king onto rook. Crazyhouse pockets sit beside each player; pick a piece and a square, or drag it, to drop it.                                                        | Play Online → _Variant_                                                     |
| Correspondence              | Seeks and challenges with 1–14 days per move. _Your games_ lists ongoing games with mini boards and _Your move_ flags; _Play_ opens one on the board and _Close game_ frees the board again. KChess checks for your turn on a timer and notifies you.    | Settings → Online play → _Check correspondence games_                       |
| Finished games              | _Analyse_ (every variant) and _Export_ (GIF, PNG, PGN; Crazyhouse pictures show the pockets).                                                                                                                                                                                                 | —                                                                           |
| Move confirmation           | A correspondence move waits on the board for _Confirm_ (Return) or _Cancel_ (Esc), as Lichess does by default. Settings → Gameplay → _Confirm correspondence moves_.                                                                                                                          |

Engine help, reviews, the explorer and cloud evaluation stay unavailable while a game is on the
board, as before; this includes an open correspondence game. Closing the correspondence game
makes them available again.

### Tournaments

The Tournaments page lists Lichess arenas, plus the Swiss events of your teams. A tournament's
panel shows its entry conditions, an entry-code field, standings that update every 15 seconds,
your rank, and Join / Withdraw. Paired games open on the board automatically, as on Lichess.
While you are in a tournament, the event stream stays open even if _Receive challenges_ is off.
Joining needs the `tournament:write` permission, so accounts connected earlier are asked to
connect again. A lichess.org tournament or Swiss link pasted into Quick Pick (⌘K) opens that
event's panel.

### Watching

The Watch page has three tabs:

- **TV**: every Lichess TV channel. The board follows the featured game with clocks.
- **Friends**: friends who are playing right now, with a Watch button. Friend cards and the
  Players page have one too.
- **Broadcasts**: live, upcoming and past broadcasts. Follow a round's live PGN feed as a grid
  of boards; any board opens with its move list and clocks, and _Analyse_ sends it to the
  analysis board.

A lichess.org broadcast link pasted into Quick Pick (⌘K) opens there too: a tournament's at
its live round, a round's at that round, with the board it names selected. A TV link
(`lichess.org/tv/rapid`) opens that channel.

Only one stream runs at a time, and watching stops while you play your own game.

### Players

The Players page looks up any account. It shows each rating with its progress, a full record
per rating category (rank, percentile, wins, draws and losses, highest and lowest rating,
streaks, best wins and worst losses) and your lifetime score against the player. From there you
can add them as a friend, challenge them or watch their current game.

### Analysis

- **Opening explorer** with the Lichess, **Masters** and **Player** databases, and the
  tablebase. Filters: speeds, rating groups, player and colour, and a since date. The Masters
  and Player databases list notable games, which open on the analysis board. A _Follow moves_
  switch refreshes the explorer as you move. A variant game is looked up in Lichess's games of
  that variant (the Lichess and Player databases).
- **Lichess cloud evaluation**: Lichess's stored deep evaluation of the position, shown beside
  the local engine, or in its place when Stockfish is off. It is off by default because every
  position is sent to Lichess. Settings → Analysis, or the cloud button in the engine panel.
- **Variant analysis**: Crazyhouse, Atomic, Antichess, Horde, King of the Hill, Racing Kings and
  Three-check games open on the analysis board in their own rules, with the Crazyhouse pockets
  beside the board, and the bundled Fairy-Stockfish analyses and reviews them (accuracy, faulted
  moves and the eval chart, judged by the variant's own wins), synced games too, in the
  background. Their mistakes become drills, played by the variant's rules (Crazyhouse drills
  with the pockets). Cloud evaluation covers standard chess and Chess960. On the command line,
  `kchess-cli analyze --variant crazyhouse <fen|moves>` and `kchess-cli review` (a PGN with its
  `Variant` header) do the same.
- **Opening names** from Lichess's CC0 opening list, bundled with the app, in every game view.
  Settings → Gameplay → _Show opening names_.
- **Lichess studies**: list your studies, open a chapter, save all chapters to the local study
  library, or send the current analysis to an existing study or a new private one. This needs
  the `study:read` and `study:write` permissions. A public study's link (a chapter's, or the
  study's for its first chapter), pasted into the command palette or Analysis → Import, opens
  that chapter on the analysis board without signing in.
- **Import a game by link**: paste a lichess.org game link (with `/black` or `#move` if you
  like) into the analysis board's import, and the game opens there. On the command line,
  `kchess-cli pgn <link>` prints a game's or a public study chapter's PGN, and
  `kchess-cli review <link|file|moves>` reviews a game (in any variant) with the engine, printing each
  side's accuracy and the inaccuracies, mistakes and blunders.
  `kchess-cli explore [masters|opening|tablebase] <fen|moves>` prints the explorer's moves for a
  position, from the tablebase when it has seven pieces or fewer (`--variant crazyhouse` looks
  up Lichess's games of a variant), and
  `kchess-cli opening <fen|moves>` names the opening it reaches, offline, and
  `kchess-cli position [--variant name] <fen|moves>` prints whose turn it is, check or mate, and the
  legal moves (with the Crazyhouse drops).
  `kchess-cli watch [channel|link]` streams a Lichess TV channel, a game or a broadcast as JSON lines.
- **Export**: animated GIF of the main line, PNG of the position, or PGN. The images are drawn
  locally with your board theme and piece set; no image service is involved.

### Puzzles

- **Open a puzzle by link**: paste a lichess.org/training link into Quick Pick (⌘K) and the
  puzzle opens on Puzzles → Daily; _Today’s puzzle_ goes back. The command line takes one too:
  `kchess-cli puzzle [id|link]`, where `hint` names the piece to move and `solution` shows the rest.
- **Analysis board after a puzzle**: a finished puzzle opens there, its solution as the main line
  and each wrong try as a side line marked _Your move_. Its themes train that theme.

### Playing without Lichess

- **Computer games**: _Game options_ starts from the standard position, a Chess960 position
  (numbered, or random), another variant (King of the Hill, Three-check, Antichess, Atomic,
  Horde, Racing Kings or Crazyhouse, played by the bundled Fairy-Stockfish, from its usual start or
  a FEN) or any FEN, with an
  optional clock (the engine's thinking time is kept within its clock), a colour and a strength.
  The editor and analysis board both have _Play Stockfish from here_, in the position's variant
  (the editor sets up positions in any of them; a Crazyhouse FEN keeps its pockets). On the command line, `kchess-cli play [level] [colour]
  [variant] [--fen FEN]`, and `kchess-cli local [variant] [--fen FEN]` over the board.
- **Over the board**: two players at one computer in any supported variant, from its usual start
  or a FEN (Chess960 by its number). It has optional
  clocks with time odds, _Turn the board_, takeback, draw and resign, and PGN, GIF and analysis
  export. The **Chess clock** mode is a full-screen clock for a real board (Space presses it,
  P pauses).
- **Blindfold** and **zen mode** on every game page. Settings → Gameplay; the eye and
  full-screen buttons switch them, and Z toggles zen mode.
- **Typed moves**: already supported. Every interactive board accepts SAN or UCI in the move box.

### Training and statistics

- **Opening trainer** (Practice → Openings): drill any saved study as White or Black. KChess
  plays the other side's moves from the study and favours the lines you get wrong. Your
  mistakes are remembered on this computer.
- **Insights**: results by colour, speed, opening family, day of week, time of day, opponent
  rating gap, game length and how games ended. It also shows streaks and average accuracy,
  inaccuracies, mistakes and blunders from reviewed games. Variant games are counted apart from
  standard chess, with a Variant choice once you have played one. **Time use** reads the clocks
  that synced Lichess and Chess.com games carry: results when your clock fell under a tenth of
  its start, your clock against your opponent's at moves 20 and 40, blunders per 100 moves short
  of time and with time to spare (in reviewed games), and time per move in the opening,
  middlegame and endgame. Everything is worked out locally from synced games.
- **Chess.com games**: a Chess.com username added in Settings → Accounts syncs its public games
  from Chess.com's public API (it needs no login) into History, the background reviews, Insights
  and mistake drills. Their speeds, variants and endings are named as Lichess names them, so both
  sites' games are read alike; bughouse and other rules KChess cannot replay are left out.

## Not possible, and why

| Feature                                                       | Why it cannot be built                                                                                                                   | What KChess offers instead                                                                                                                           |
| ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Bullet and UltraBullet games, and Blitz seeks                 | Lichess's Board API, which third-party clients must use, refuses them. Public seeks need Rapid or slower; direct challenges allow Blitz. | The time-control form explains the limit. Tournaments list only arenas whose games the Board API can play.                                           |
| Most arenas                                                   | Most arenas are Bullet or Blitz, which the Board API cannot play.                                                                        | _Playable here_ filters to Rapid and Classical events; _All_ shows the rest, marked.                                                                 |
| Cancelling a public correspondence seek                       | The API creates the seek but has no endpoint to withdraw it.                                                                             | The form warns about this; a direct correspondence challenge can be withdrawn.                                                                       |
| Notifications for correspondence moves while KChess is closed | Lichess cannot reach a closed desktop app, and the event stream sends no correspondence moves.                                           | KChess checks while open (every 2–60 minutes, as set).                                                                                               |
| Lichess's rematch button                                      | The API has no rematch endpoint.                                                                                                         | _Rematch_ sends a challenge with the same settings and colours swapped. The opponent sees an ordinary challenge. Incoming rematch offers work fully. |
| Puzzle Racer                                                  | `/api/racer` only creates a race that must be played on lichess.org. Leaving the app to finish a task is not acceptable here.            | Storm, Streak and Rush, played locally.                                                                                                              |
| Requesting Lichess's server analysis                          | The API cannot ask Lichess to analyse a game.                                                                                            | Local Stockfish reviews, plus any Lichess analysis already attached to synced games.                                                                 |
| Private messages (inbox)                                      | The API can send a message but not read the inbox.                                                                                       | —                                                                                                                                                    |
| Simuls                                                        | They can be listed, but not joined or played.                                                                                            | —                                                                                                                                                    |
| Forums, blogs, team management                                | No API, or out of scope for a chess client.                                                                                              | Teams' Swiss events appear under Tournaments.                                                                                                        |
| Lichess _Learn_ and _Practice_ lessons                        | No API. The lesson content is lila's AGPL code and data, so it would have to be rebuilt lesson by lesson.                                | Practice drills, the opening trainer and mistake practice.                                                                                           |
| Editing Lichess studies chapter by chapter                    | The API replaces a chapter's moves and tags as a whole; comments cannot be merged. Two-way sync would lose edits.                        | Import chapters, and send analyses as new chapters.                                                                                                  |
| Moves of other people's live games without delay              | Lichess delays game streams by three moves to stop cheating.                                                                             | Watch shows the delayed position and says so.                                                                                                        |
