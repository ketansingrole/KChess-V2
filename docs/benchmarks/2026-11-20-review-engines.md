# Game review on an engine for each thread: evaluated, not adopted

**Status: evaluated, not adopted.** Built and measured in
[#290](https://github.com/ketansingrole/KChess-V2/pull/290), closed without merging; its branch
`claude/parallel-review` holds the code. A review someone asks for finished in under half the
time on four one-thread engines, but its scores were nearly twice as far from a deeper review's.
At equal accuracy the two designs cost about the same engine work.

## The idea

A review scores every position of a game to depth 18 on one Stockfish with every search thread
(up to four), one position at a time. To a set depth, more threads search more: on the bundled
Stockfish 19, with the commands the review sends, over an 80-move game (81 positions):

| Setup                                       | Nodes | Engine CPU |
| ------------------------------------------- | ----: | ---------: |
| 1 engine, 4 threads, first position to last | 47.0M |     52.3 s |
| 1 engine, 1 thread, first position to last  | 18.2M |     22.4 s |
| 4 engines, 1 thread each, four equal parts  | 19.0M |     23.4 s |

So #290 gave each thread an engine of its own and dealt the positions in runs of five from a
queue the engines shared, with no more engines than one per 2 GiB of memory, a fall back to one
engine where a thread was too slow to reach the depth in time, and the deep pass searched from
the last position to the first (9% fewer nodes on one thread). Through the review itself
(`Reviews::request_review`, both passes, the same game) it took 5.1–5.6 s against `main`'s
10.7–13.6 s, with 20M nodes against 57–62M.

## Why it was not adopted

Reaching a depth sooner is not reaching the same answer sooner. Three 80-move games Stockfish
played against itself at depth 8, every seventh move at depth 1 so the games have errors, each
review compared with a depth-24 review that searched every position from an empty hash. The error
is the difference in winning chance, in points of 100, averaged over the 243 positions; the CPU
time is the engines' and sets the time a review takes, since both designs keep four cores busy.

| Setup                                 | Depth | Average error | Engine CPU |
| ------------------------------------- | ----: | ------------: | ---------: |
| 1 engine, 4 threads (`main`)          |    18 |          0.87 |      233 s |
| 1 engine, 4 threads                   |    17 |          1.01 |      129 s |
| 1 engine, 4 threads                   |    16 |          1.13 |      100 s |
| 1 engine, 4 threads                   |    15 |          1.34 |       72 s |
| 4 engines of 1 thread, first to last  |    22 |          0.96 |      157 s |
| 4 engines of 1 thread, first to last  |    20 |          1.13 |       87 s |
| 4 engines of 1 thread, first to last  |    18 |          1.38 |       47 s |
| 4 engines of 1 thread, last to first (#290) | 18 |       1.54 |       45 s |

- #290 did a fifth of `main`'s engine work and had nearly twice its average error.
- At equal accuracy the designs cost about the same: `main` at depth 16 and four engines at
  depth 20 both come to 1.13, for 100 s and 87 s. At `main`'s accuracy the four engines save
  nothing that these runs can show.
- What the four engines offered is what a lower depth on `main` offers. Depth 17 is 45% less work
  for an error of 1.01 instead of 0.87: a choice between speed and accuracy, not a free gain.

### What makes the difference

The same three games at depth 18, one thing changed at a time:

| Setup                                          | Average error | Nodes |
| ---------------------------------------------- | ------------: | ----: |
| 1 engine, 4 threads, first to last (`main`)    |          0.87 |  502M |
| 1 engine, 4 threads, last to first             |          1.13 |  456M |
| 1 engine, 1 thread, first to last              |          1.22 |   83M |
| 1 engine, 1 thread, last to first              |          1.45 |   86M |
| 4 engines of 1 thread, last to first, hash 16  |          1.40 |  104M |
| 4 engines of 1 thread, last to first, hash 64  |          1.41 |  104M |
| 4 engines of 1 thread, first to last, hash 16  |          1.38 |  100M |
| 4 engines of 1 thread, first to last, hash 64  |          1.37 |   97M |

- **The threads.** The nodes four threads add at a depth are not waste: they are what makes the
  score better. Depth names how far the search went, not how much it saw.
- **The order.** Searching from the last position to the first is less accurate on one engine,
  with four threads or one, for about the same work. It saves nodes because the engine meets
  positions it has already scored, and it trusts them.
- **Not the hash**: 16 or 64 MiB an engine made no difference.

The labels a player sees moved less than the scores: of 240 moves, the inaccuracy, mistake and
blunder labels differed from the depth-24 review's on 7–11 for `main` and 8–12 for the four
engines, which is within what two runs of `main` differ by.

## Limits

Each setup ran once per game, on a Mac that other builds kept busy, so differences of a tenth of
a point are noise; node counts and CPU time do not depend on the load much, wall times do. Three
games from one engine's self-play are not every kind of game. Nothing was measured on Windows or
Linux.

## If reviews should be faster

- Lower the depth on purpose, and say what it costs: the table above gives the price of depths
  15 to 17.
- Any other design is judged the same way: scores and labels against a deeper review searched
  from an empty hash, at equal engine time. Time to a depth says nothing about it.
