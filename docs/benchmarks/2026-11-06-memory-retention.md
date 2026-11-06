# Memory retention: what a long session keeps

**Status: complete.** Each benchmark repeats one thing a session does for hours and measures what
the repetitions leave behind, before and after the fixes on the same Linux machine (4 cores).

- The core and the service: a counting global allocator gives the live heap exactly, and the
  runtime's count of live tasks shows work left running. Each scenario warms up first, so caches
  that are bounded by design are full before it measures; it then fails when the cycles keep more
  than their allowance, so these are also regression tests.
  `cargo test -p kchess-core --test memory -- --nocapture`,
  `cargo test -p kchess-service --test memory -- --nocapture`.
- The app: the Over the board page driven headless, with resident memory read from `/proc`.
  `cargo test -p kchess-gpui e2e::memory -- --ignored --nocapture`.

## Results

| Scenario                                    | Cycles | Before                             | After                       |
| ------------------------------------------- | -----: | ---------------------------------- | --------------------------- |
| App: turned clock face shows something new  |    200 | **+688.7 MiB** (3.5 MiB per face)  | +0.2 MiB                    |
| Core: a client analysing, then leaving      |    100 | +42.2 KiB, +100 tasks (432 B each) | +0.1 KiB, no tasks          |
| Core: a profile opened and closed           |     20 | +34.3 KiB, +20 tasks (1.7 KiB each) | +0.3 KiB once, no tasks    |
| Core: analysis started and stopped          |    100 | +0.1 KiB                           | +0.1 KiB                    |
| Core: client connects, asks, leaves         |   1000 | +0.3 KiB                           | +0.3 KiB                    |
| Core: computer move                         |    100 | not measured                       | +0.1 KiB                    |
| Service: calls on one connection            |   5000 | +0.1 KiB                           | +0.2 KiB                    |
| Service: 64 calls in flight at once         |    200 | +4.3 KiB once, then flat           | +4.3 KiB once, then flat    |
| Service: window connects and leaves         |    200 | +0.1 KiB                           | +0.1 KiB                    |

A few hundred bytes that stay flat across batches are allocator and cache capacity, not
retention. Each scenario prints its figure after every batch: a leak grows by the same amount
every batch (the turned clock: 0 → 171 → 343 → 514 → 689 MiB).

## What kept memory, and the fix

**Turned clock faces (the app, unbounded).** GPUI cannot rotate text, so a clock face turned for
the player across the table is drawn as a picture. Every change of the face (each second while
the clock runs, each tenth of a second in the last ten) made a new PNG, which GPUI decoded and
kept in its asset cache, with its texture, for the rest of the session: some 3.5 MiB each at the
test's scale, so roughly 12 GB an hour with a turned clock running. The face is now drawn from its
own image, outside the asset cache, and the face it replaces (or a face turned back upright)
releases its texture. `e2e::memory::a_turned_clock_face_releases_the_picture_it_replaces`.

**Piece images (the app, unbounded on the GPU).** The board keeps its rasterized pieces for eight
sizes and drops older ones, but the dropped images kept their textures in the window's atlas, so
each size a window passed through while being resized stayed on the GPU: twelve textures of the
square's size (about 300 KiB at 80 px squares). They are now released after the frame.
`e2e::memory::piece_sizes_a_resize_left_behind_are_released`. The headless test atlas holds no
pixels, so this one is checked by atlas entries rather than bytes.

**Preview boards and timers (the app, small, unbounded).** The Openings explorer kept a preview
board for every move tile of every position stepped through, the Online page one for every game
that was ever ongoing, Watch TV one for every game a channel showed, and Puzzles a finished
auto-next timer per puzzle solved. Each now keeps only what it shows.

**A client leaving mid-analysis (the core, held two minutes).** Its engine was killed, then the
analysis's end armed the two-minute idle countdown that only closes the engine, which kept the
analysis's state alive with it. No engine, no countdown.

**A closed core's battery check (the core, held fifteen seconds).** The refresh loop held the
core's capabilities (its connections and host) until its next tick. It now ends when the core
closes or is dropped.

**Cancelled calls (the core).** A call a socket client cancels or abandons has its task aborted,
so cleanup written after an `.await` never ran. A cancelled Lichess or Chess.com sync left its
entry behind, and every later sync of that player joined it and failed at once with "Sync was
cancelled." until the player was removed. Host requests and engine installs left entries too. The
cleanup now runs on drop (`tasks::on_drop`).
`chesscom_sync::a_cancelled_sync_does_not_fail_the_next_one` fails before the fix.

**Service call bookkeeping (code review, not reproduced).** The service spawned each call, then
registered its abort handle: a call another worker finished in between removed nothing, and the
late entry (with the finished task it keeps allocated) stayed until the window disconnected. The
call is now registered before it can run. The benchmarks never lost that race on this machine.

## Left as they are

Bounded by what the person does, or a few bytes each: the board editor's undo history, broadcast
cover pictures (kept for the session by design), correspondence turn flags and review keys per
game, the back history, one closure per page rebuild, and the per-player sync epochs, which must
outlive a removal to reject a stale sync.
