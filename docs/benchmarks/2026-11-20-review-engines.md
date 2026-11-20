# Game review: an engine for each thread instead of one engine with them all

**Status: complete on one Mac; not measured on Windows, Linux or a low-end computer.** A review
someone asks for takes under half the time: 5.1–5.6 s against 10.7–13.6 s for an 80-move game,
with a third of the positions searched.

## Where the time went

A review scores every position of a game to depth 18. It gave one Stockfish every search thread
(up to four) and one position at a time. A search to a set depth gains little from more threads:
the extra threads mostly search positions one thread would have skipped. Measured on the bundled
Stockfish 19 with the commands the review sends (81 positions, depth 18):

| Setup                                           | Positions searched | Engine CPU |
| ----------------------------------------------- | -----------------: | ---------: |
| 1 engine, 4 threads, first position to last     |              47.0M |     52.3 s |
| 1 engine, 1 thread, first position to last      |              18.2M |     22.4 s |
| 1 engine, 1 thread, last position to first      |              16.6M |     19.9 s |
| 4 engines, 1 thread each, four equal parts      |              19.0M |     23.4 s |

Four threads searched 2.6 times what one did for the same depth, so they finished only about 1.7
times sooner. Four engines of one thread search what one thread does, four positions at a time.
Going from the last position to the first saves another 9%: the engine has just searched the
position its next one leads to.

## The change

- **A review asked for runs on an engine for each search thread** (`engine/review.rs`, `round`),
  each with one thread, under one lease of the engine budget. The positions are dealt in runs of
  five that follow one another in the game, from a queue the engines share: an engine on a slower
  core takes fewer runs instead of holding the others up.
- **The deep pass goes from the last position to the first.** The quick pass still goes from the
  first, as it shows. Automatic reviews (one engine, one thread) gain the 9% too.
- **No more engines than the memory has room for** (`engine::scheduler::engines_within`): one
  per 2 GiB of the computer's memory, read from the OS (`memory.rs`). Engines that share the
  threads share the hash too (64 MiB in all). With room for fewer engines than threads, each gets
  more threads: two engines of two on a 4 GiB computer with four threads.
- **A computer too slow for it keeps the old way.** When one of several engines runs out of its
  five seconds before depth 18, the pass goes again on one engine with every thread for the
  positions still short, so no review ends shallower than it did before.
- **The other engines close when their work ends**: at the end of the queue, when an automatic
  review follows, and when a pass falls back to one engine. The first engine stays for a minute
  as before.
- A game asked for while its automatic review runs now gets the quick pass and the engines of a
  review asked for from then on; before, it only got more threads.

### Run length

Four engines, last position first, by how many positions an engine takes at a time:

| Run of    | Positions searched |
| --------- | -----------------: |
| 3         |              21.1M |
| 5         |              18.6M |
| 8         |              18.3M |
| 12        |              17.9M |
| 20 (even) |              17.4M |

Runs of five cost 7% over four even parts and let a slow engine fall behind by at most one run.

## Results through the review itself

`review_speed_by_engines` (`tests/core/engine_review_stockfish.rs`) reviews the same game through
`Reviews::request_review`, both passes, on a scheduler with four threads. `main` at `67532e3e`
against this change, two alternating rounds; the positions searched are read from what the
engines printed.

| Build                                    | Time, s      | Positions searched |
| ---------------------------------------- | ------------ | ------------------ |
| `main`: 1 engine, 4 threads              | 10.7, 13.6   | 62.2M, 56.6M       |
| 4 engines of 1 thread (8 GiB and up)     | 5.1, 5.6     | 20.0M, 19.2M       |
| 3 engines of 1 thread (6 GiB)            | 7.9, 4.4     | 21.2M, 19.3M       |
| 2 engines of 2 threads (4 GiB)           | 7.7, 6.0     | 32.6M, 35.6M       |
| 1 engine, 4 threads (under 4 GiB)        | 16.7, 7.6    | 52.8M, 51.8M       |

Every position reached depth 18 in every run. Other sessions were building on the Mac while this
ran (its load fell from about 35 to 15 during the run), so the times vary; the positions searched
do not depend on the load.

## What it costs

One engine's memory footprint while it searches is 157 MB on one thread and 239 MB on four
(`footprint`, with either 16 or 64 MiB of hash). Four engines take about 630 MB, some 390 MB more
than before, for the seconds a review asked for runs; hence one engine per 2 GiB of memory.

## Not measured

- Windows and Linux, and a computer with two cores or little memory. A computer with one search
  thread, or under 4 GiB, reviews as before.
- A chip with fast and slow cores under no other load: the shared queue is there for it, but how
  far the engines fall apart was not timed.
- A quiet Mac: the times above are from a busy one.

## Reproduce

```bash
KCHESS_ENGINE_PATH=<stockfish> cargo test -p kchess-core --test core review_speed -- --ignored --nocapture
KCHESS_REVIEW_ENGINES=4 KCHESS_ENGINE_PATH=<stockfish> cargo test -p kchess-core --test core review_speed -- --ignored --nocapture
```
