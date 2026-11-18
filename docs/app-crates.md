# The app's crates

`apps/gpui` was one crate of about 105,000 lines (186 files, 14,500 of them its end-to-end
tests). This is the plan for splitting it into crates, as Zed does (`ui`, `theme`, `settings`, a
`*_ui` crate per feature) and as GPUI Kit's own guide recommends (`base` → `component` → the
app, with feature crates behind an app shell), and what each step measured.

## What a split can buy, measured

2026-11-18, a Linux container with 4 cores, the `dev` profile (`cargo build -p kchess-gpui`,
everything built once first; each figure is the mean of two runs):

| Edit | `cargo build` | `cargo check` |
| --- | --- | --- |
| A string in a page's function (`pages/insights.rs`) | 9.2 s | 3.6 s |
| The same in `theme.rs` | 9.3 s | 3.5 s |
| Only the file's modified time (`touch`) | 9.2 s | — |
| A new `pub const` in `theme.rs` or a page | 9.6–10.2 s | — |
| A field added to `theme::Palette` (named in 82 files) | 26 s | — |
| The app crate from scratch (its dependencies built) | 45.7 s | — |

Where an incremental build of the app crate goes (`rustc -Z time-passes`, the string edit, 8.0 s
in all): macro expansion and name resolution 1.0 s, type checking and borrow checking under 1 s,
monomorphization collection and partitioning 2.0 s, code generation 2.4 s, saving the incremental
state 1.7 s, linking 1.5 s. From scratch: type checking 6.9 s, borrow checking 6.0 s, coherence
2.8 s, monomorphization collection 11.4 s, code generation 28 s.

What follows from it:

- **An edit costs the same anywhere in the crate.** The incremental floor (expansion, resolution,
  collection, partitioning, saving the state) grows with the crate's size, not the edit's, so a
  one-word change in a page costs what touching the file costs.
- **A crate costs at least its whole floor whenever a crate it depends on changes.** Cargo
  rebuilds every dependent of a changed crate; incremental compilation reuses some of its work,
  but not that floor. So moving the shared pieces out (`kchess-ui`) speeds up nothing by itself:
  an edit there still rebuilds the whole app after it (and, measured below, slower). The gain comes when the pages leave too, and the crate an edit
  in a page rebuilds after it, the app shell, is small.
- **Changing a widely used type is the expensive edit** (26 s for a field of `Palette`): that is
  the from-scratch work of every function that names it. Split, the crates naming it type-check
  in parallel instead of one after another on one thread.

Each step below measures the same edits again, so a step that buys nothing is seen.

### After step 1 (`kchess-ui`)

The same machine and edits, `cargo build`, the mean of two runs:

| Edit | Before | After |
| --- | --- | --- |
| A string in a page's function | 9.2 s | 8.3 s |
| A string in the theme (now `crates/kchess-ui/src/theme.rs`) | 9.3 s | 19.5 s |
| Only a page's modified time | 9.2 s | 8.1 s |
| A field added to `theme::Palette` | 26 s | 26.7 s |

An edit to code that moved out became twice as slow: on top of building `kchess-ui`, the app
redoes far more of its own work after a change in a crate it depends on than after the same
change in itself. Shared code that is edited often costs more in its own crate until the crates
depending on it are small, so the later steps have to shrink the app for any step to pay off,
and a step should move code that rarely changes before code that often does.

## The order

Moving code out of the app makes edits to it slower until the app is small (step 1 above): an
edit in a crate the app depends on cost 19.5 s where the same edit in the app cost 9.3 s. So
code leaves in the order it changes least, shrinking the app fastest for the least cost, and the
code that changes most stays in the app longest, where an edit rebuilds one crate.

How often each part changes, from the app's history since 2026-11-09 (31 commits, leaving out the
3 that touch more than 40 files; lines changed over the part's size). That is nine days of history,
so read it as a ranking, not a rate, and check it again before each step:

| Changes most (stays in the app longest) | Changes least (moves first) |
| --- | --- |
| Settings (0.07), Online (0.07), `core` (0.16), `ai` (0.24), `position_engine` (0.32), `puzzle_board` (0.13), `quick_pick` (0.29), cards (0.11), the end-to-end tests | the generic widgets (0.01–0.03), the log and assets (0.03–0.04), `dates`, the board (0.02), `game` (0.01), practice (0.01), watch (0.01), history (0.01), puzzles (0.01), local, home and studies (0.00–0.01) |

Each step is one pull request, each green on its own.

1. **`crates/kchess-ui`, the design system** (done, #254). The theme and its colors, motion and
   the line chart. What it needs from the app is passed in: where log lines go
   (`kchess_ui::Logger`, set by the app at start) and which themes Settings chose
   (`theme::ThemeChoice`); the app's `theme::apply` reads Settings and hands them over. It depends
   on GPUI Kit, `kchess-contracts` and `kchess-domain`, never on the app or the core.
2. **The generic widgets that rarely change** into `kchess-ui`: badges, dialogs, the panel, pills,
   status, the widgets' shared traits (`KeyboardFocus`, `Pressable`, `LiveRegion`), `dates` and
   the `icon` they draw with (the icons stay embedded in the app; `icon` only names a path). Cards,
   `highlight` and `quick_pick` change more and stay; so do the domain widgets (moves, players,
   setting rows) until a board crate exists.
3. **The app's services as a crate** (`kchess-app`), the part of them that rarely changes: view
   preferences (`prefs`), settings, the log, accounts, sound, timers, and navigation as a trait or
   global that the shell implements, so a page opens another page without naming the shell or
   that page. `core.rs` and `online.rs` change often: the crate reaches them through a trait the
   app implements until they settle.
4. **The board** (`kchess-board`): the board, its looks, pockets and previews, `game`, notation,
   and the domain widgets left from step 2.
5. **Feature crates for the pages that rarely change**, the largest first: practice (9,000 lines),
   puzzles (4,900), watch (4,300), history (3,000), studies, local and home. Where a page names
   another (`pages::analysis` from puzzles and watch), it opens it through step 3's navigation.
6. **The rest, once the app is small enough that its crates pay off**: analysis, Settings, Online,
   tournaments and the pieces they share (`position_engine`, `ai`, `puzzle_board`, cards and
   `quick_pick`). Measure first: by then an edit in the app should cost well under the 9 s it
   costs today, and a page that keeps changing may be cheaper left in it.
7. **The app shell**: `apps/gpui` keeps `main`, the window, the menu, the updater, notifications
   and the end-to-end tests.

## What each new crate needs

- A line in the root `Cargo.toml`'s `[workspace.dependencies]` and `members`, and `kchess-hack`
  as every member has it (`cargo hakari generate`).
- Its role in xtask's member table (`tooling/xtask/src/members.rs`): a GPUI crate the app is
  built from is `Role::Ui`, outside the coverage totals as the app is (the app's end-to-end suite
  checks its views), its tests run with the headless members' (Linux in `check`, Windows and
  macOS in `headless`).
- Its sources in xtask's `boundaries.rs` `APP_SOURCES`, so the app's rules of look (spinners,
  named selects, corners, clocks) and the core's boundaries hold there too.
- No `unwrap` or `expect` outside tests: a library is linted for them (`cargo xtask check`),
  where the app's binary is not.
- A place in the Mutants workflow's package list (`.github/workflows/mutants.yml`), with GPUI's
  system libraries installed for it there; CI's `mutants` job installs them for every UI crate.
- Its moved lines' mutants killed: a pull request's mutants run (`cargo xtask mutants`) tests a
  UI crate's changed lines with its own tests, not the app's, so the code it takes along needs
  tests of its own.
