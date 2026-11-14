# Mutation testing

[cargo-mutants](https://mutants.rs) changes the code one small edit at a time (a `>` becomes
`>=`, a function returns `Default::default()`, a match arm goes) and runs the package's tests
against each change. A change the tests still pass is a *missed* mutant: code whose behaviour no
test pins.

```bash
cargo install cargo-mutants cargo-nextest
```

```bash
cargo mutants -p kchess-service --test-tool nextest
```

The results land in `mutants.out/` (`missed.txt`, `caught.txt`, `timeout.txt`,
`unviable.txt`). After writing tests, `--iterate` tests only the mutants that were missed last
time and those in changed code.

## Every pull request

New code arrives with its mutants killed. CI's `mutants` job runs

```bash
cargo xtask mutants
```

on each pull request: cargo-mutants mutates only the lines changed since `main` (`--in-diff`,
committed or not), tests each mutant with the tests of its own package, and fails when one
survives or times out. Mutants that were already missed in untouched code are the weekly run's,
so a change never has to pay off the whole backlog at once.

CI first counts a pull request's mutants (`cargo xtask mutants --plan`, which builds nothing),
then splits them over a runner per 15, up to 20, as many as the account runs at once
(`--shard k/n`), one at a time on each, in the checkout itself (`--in-place`), built
incrementally from the crates.io dependencies each push to `main` keeps built, and gives each
runner up to six hours, GitHub's limit, so a large refactor's mutants are tested too. A change
with no mutants starts no runner. The app's mutants (`apps/gpui`) each rebuild the app, so
`cargo xtask mutants` leaves them out of a run of changed lines (pull requests, the pre-push hook);
`--app` tests the app's alone, as the weekly run's `gpui` job does for the lines the past week
changed in the app.

The pre-push hook (`cargo xtask install-hooks`) runs it before every push of a branch, on the
lines new since that branch's last push, and refuses the push while a mutant survives: a survivor
costs minutes here rather than a whole CI run. It tests them in the checkout's own mutants tree
(`.data/mutants-tree`, a detached worktree set to the pushed commit), never in the files you are
editing; the tree stays built, so a push rebuilds only what changed since the last one. It counts
them first (as `--plan` does): below 30 it tests them as CI does, one at a time in place;
from there, half the CPUs' worth at once (at most six), each job in cargo-mutants' own copy of the
tree with its build (`--copy-target`, cloned rather than copied on APFS), so each rebuilds only the
workspace's crates, not their dependencies: 25 kchess-domain mutants took 174 s five at once
against 294 s in place. A second push from the same checkout waits for the first, and whatever
an interrupted run left in the tree is reset before the next. Results land in the checkout's
`mutants.out/`. It skips a push of anything but a clean HEAD, and
clones without cargo-mutants; `KCHESS_SKIP_PRE_PUSH=1 git push …` skips it on purpose. When CI's
`mutants` job fails, the `mutants-diff-<k>`
artifact's `missed.txt` and `timeout.txt` name the survivors. To work on them locally,
`--base <ref>` diffs against another commit, `--all` tests every mutant instead of the changed
lines', `--in-place` builds and tests them in the checkout itself (only one nobody is editing),
and arguments after `--` go to cargo-mutants
(`-- -j 4`, `-- -p kchess-domain`, `-- --file 'crates/kchess-core/src/engine/**'`).

Code that only moved is not a change: a block Git's `--color-moved` marks as moved (at least
20 letters and digits, whatever its indentation) goes to cargo-mutants as context, so splitting
a file or moving a function into a module tests nothing again, while an edited line inside the
moved code still is a change. Commit a move first: an untracked file is compared with nothing.

## What to do with a missed mutant

1. Write the test it shows is missing, at the level that observes the behaviour: a unit test, a
   CLI or service test over a scratch profile, or a local fixture (the CLI's tests serve a
   stand-in Lichess and use the core's scripted engine, `kchess-fake-uci`).
2. When the code is hard to observe, make it observable rather than test around it: the voice
   helper binds libvosk from any symbol source, so its tests bind in-process fakes; the profile's
   layout (`kchess_service::Layout`) takes the environment as a parameter, and the CLI's host
   answers take the browser opener.
3. When no test can tell the mutant apart because the program behaves the same (an *equivalent*
   mutant), exclude it in `.cargo/mutants.toml` with the reason. Never exclude one because a
   test is missing. Prefer exclusions by function name to line numbers, which move.

## Weekly runs

`.github/workflows/mutants.yml` runs a package in eight shards every Saturday (the rules) and on
demand for any package, through `cargo xtask mutants --all --in-place` as a pull request's run
does. Its `gpui` job, on the same schedule, tests the mutants of the app's
lines changed in the past week, which pull requests leave out. The rules and the core take hours on one machine: about 20 seconds of
CPU per mutant for the rules' 7,000, more for the core's 6,300 and the app's 12,000, whose every
mutant rebuilds the app.

## Mutants no test here can kill

These stay in the reports on purpose; each is listed with the reason.

- **Windows-only code** (`kchess-service`'s named pipes and owner checks, `launch`'s creation
  flags): not compiled on Linux or macOS, so a mutant there builds and passes on them. They can
  only be caught by a run on Windows. Likewise the import of the old Mac database
  (`Layout::config`'s `legacy_database_path`, kchess-service) is caught only on macOS, where its
  test checks it.
- **Stack sizes** (`kchess-service`'s and the CLI's 8 MiB core threads): a smaller stack changes
  nothing until a call overflows it, and no deterministic test overflows a 1 MiB stack on
  Linux or macOS (the case it guards, an embedded sign-in in a Windows debug build, needs
  Windows).
- **The service's slow-reader cleanup** (`server.rs`, `handle`'s last `if`): whether a writer
  stuck on a client that stopped reading is cancelled now or left until the client reads again,
  the client receives the same frames and the same end.
- **Per-window `suspend`** (`server.rs`): the difference from stopping every window's work shows
  only in an engine search or a puzzle download of another window.
- **The CLI's own list of saved games** (`play.rs`, `written`): it only feeds the Game history
  rules within one run, which find an entry by its id with or without the duplicates a mutant
  leaves; the saved library is the same.
- **The CLI's `close` and `open_browser`**: the process exits right after `close`, with the
  core's writes already on disk (excluded in `.cargo/mutants.toml`); `open_browser` starts the
  desktop's opener, which a test must not do.
- **The AppImage step's binding** (`package/linux.rs`, `appimage`): it hands the real pins, the
  `target/package` cache and `download` to `appimage_with`, which its test runs whole with a
  stand-in appimagetool. The binding downloads from GitHub and runs the real tool, so only
  `cargo xtask package` on Linux (CI's package job) sees it; it is excluded in
  `.cargo/mutants.toml`.
- **The Fairy-Stockfish and voice-runtime bindings** (`fairy.rs`, `build`; `voice.rs`, `stage`):
  they hand the real pins, `target` folders and `download` to `build_with` and `stage_with`,
  which their tests run whole with stand-in archives. The bindings download from GitHub (and
  `build` compiles Fairy-Stockfish), so only `cargo xtask fairy-stockfish`, `voice-runtime` and
  `package` see them; they are excluded in `.cargo/mutants.toml`.
- **The tooling's other bindings and tool runs** (xtask): the steps of `check` that run cargo
  over the workspace, what talks to GitHub through `gh`, the platform packagers (snapcraft and
  the macOS tools), and the bindings of the real environment to the functions their tests run
  whole (`engines::run_with`, `update_key::sign_with`, `release::metadata`, `voice::outdated`,
  `app::startable`, `mutants::warm_commands`, `check::scope_report`), and `release-version`,
  which rewrites the repository's version and notes from origin's tags.
  Each is excluded in `.cargo/mutants.toml`, naming the CI job or command that runs it.
