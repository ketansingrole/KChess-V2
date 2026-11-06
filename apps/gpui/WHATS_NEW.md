# What's new in KChess

The app shows these notes once after an update, and keeps them under Settings → Updates →
What's new and in Quick Pick. Write what a change brings under `## Unreleased`, one line a player
understands: `- **Title**: what it does for them.` `cargo xtask release-version` turns that heading
into the new version and its date, and refuses to release with nothing under it.

## Unreleased

- **What's new**: after each update KChess shows what changed, and keeps every release's notes under Settings → Updates.
- **Crazyhouse**: play it against the computer, over the board and on Lichess, dropping captured pieces from the pockets beside each player.
- **Guess the move**: a Practice drill that plays through a master game or your own PGN and asks you for each move.
- **Spaced repetition**: the repertoire trainer brings back the lines you miss when they are due.
- **Chess.com**: import your Chess.com games, and Home shows a Chess.com player's activity.
- **Time use in Insights**: how you spend your clock, from the clock times your synced games carry.
- **Studies from a file**: drop a PGN with several games on any page to open it as a study.
- **Keyboard and screen reader play**: move pieces on the board from the keyboard, with every square announced.
- **Variants everywhere**: the board editor sets up any variant, mistakes from variant games become drills, and synced Lichess variant games are reviewed.
- **A clearer Home**: games say who won, a finished game shows its result, and rating cards switch the chart.
- **Calmer motion**: KChess follows your system's Reduce motion setting, and what you open from the keyboard appears at once.
- **Steadier updates**: an interrupted download picks up where it stopped, even after a restart.
- **Fixes**: memory leaks closed, security hardening from the November audit, and faster busy pages.

## 2026.11.3 (2026-11-04)

- **Clearer Lichess errors**: an error says what happened, and the page offers what can fix it.
- **Watching games**: a watched game starts without waiting on Connecting, its clock runs between refreshes, and a finished game ends quietly.
- **Following players**: the timeline shows what the players you follow did, and marks those with games to sync.
- **Polish**: large text is drawn whole, and switches and banner actions line up with their labels.
