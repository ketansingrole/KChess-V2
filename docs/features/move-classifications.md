# Analysis move classifications

Run **Review game** on the analysis board to classify its main line. Selecting a
reviewed move shows its label as a badge on the corner of its destination square and
marks it in the move list. Variations and unreviewed positions have no engine label.
Existing PGN annotations remain intact. Live human games never show this assistance.

Error labels retain the existing Lichess winning-chance model and any judgments
supplied by Lichess. Positive labels use the same winning-chance curve:

- **Best (★):** the engine's first choice, losing at most 2 percentage points.
- **Excellent (✓):** another move losing at most 2 percentage points.
- **Good (•):** loses more than 2 but less than 5 percentage points.
- **Brilliant (!!):** meets Best plus the sacrifice checks below.

Brilliant candidates require evaluations at depth 18 or deeper both before and
after the move. The mover's evaluation before the move must be between −1 and +2
pawns; afterward it must be at least −0.5 or a forced mate for the mover. The move
must offer a bishop, knight, rook or queen with more than one legal choice. The
opponent's best-defense principal variation must capture that piece, and after
the mover's next reply the material balance must still have fallen by at least
two pawns. Equal trades and immediate material recovery therefore do not qualify.

These are conservative KChess criteria, inspired by Chess.com's sound-sacrifice
concept, not its proprietary or rating-dependent classifier. Depth is a confidence
filter, not a proof. Sacrifices declined by best defense, delayed sacrifices and
long combinations can be missed. A candidate remains Best until the review has
enough evidence; missing scores never receive a positive label. Error counts,
accuracy scores, persisted PGNs and mistake practice keep their existing semantics.
