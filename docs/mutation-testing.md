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
`--app` tests the app's alone, each with the app's whole suite, and the weekly run tests the
lines the past week changed in the app, each with the tests that run it (below).

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
does. The rules and the core take hours on one machine: about 20 seconds of CPU per mutant for
the rules' 7,000, more for the core's 6,300 and the app's 12,000, whose every mutant rebuilds the
app.

### The app's week

Every Sunday (or by hand: `gh workflow run mutants.yml -f app_week=true`), the `gpui-plan` and
`gpui` jobs test the mutants of the app's lines changed in the past week, which pull requests
leave out. A busy week has about 4,000 of them. Each used to run the app's whole suite, 669 tests
and about 2 minutes on a runner, plus a 17-second rebuild of the app: some 150 runner-hours, so
on 2026-11-14 all four shards were cancelled at six hours with nothing reported. Yet a mutant can
only show in a test that runs its line, and most lines are run by a handful of tests.

So `cargo xtask mutants --app-plan --base <ref>` (`tooling/xtask/src/app_mutants.rs`) first
measures which tests run which lines: it builds the app's tests with coverage of the app's own
code (`cargo rustc … -C instrument-coverage`, the dependencies left as the shards build them),
runs each test alone with its own profile, and reads each one's covered lines with the
toolchain's `llvm-cov` (`rustup component add llvm-tools-preview`). Each mutant gets the tests
that ran a line of its span; half of them need 5 tests or fewer, and the week's estimate fell
from about 150 hours to under 40. A mutant no test runs is not built at all: it is listed in
`uncovered.txt`, a test to write as surely as a missed one. The plan groups the mutants that
share their tests into chunks of up to fifteen estimated minutes, and spreads the chunks over as
many shards as four hours each takes (at most 16), the longest first, onto the least loaded. On
this Mac the plan takes about a minute; on a runner, its first build too.

Each shard (`cargo xtask mutants --app-shard <k>`) builds the app once, then runs cargo-mutants
once per chunk, in place, with `--baseline skip` (the plan ran every test) and a nextest filter
of the chunk's tests (`-E 'test(=…) | …'`). It picks the chunk's mutants by their exact names
(`--re`) and by a diff that marks only their lines (`--in-diff`): cargo-mutants 27.1 never checks
a struct field's "delete field" mutants against names, so by name alone every chunk tested all
of its files' (and `.cargo/mutants.toml`'s exclusions do not reach them either). Its `mutants.out` adds up the chunks' lists
(`caught.txt`, `missed.txt`, `timeout.txt`, `unviable.txt`), each chunk's own logs under
`chunks/`. A shard starts no chunk past five hours, so it always reports within the job's six;
anything it did not reach is listed in `untested.txt`, and the plan's next week needs more
shards, not a longer limit.

The selection trusts that a test which never runs a line cannot see a change to it. A test whose
path depends on timing may run a line in one run and not the next: two runs of the plan here
disagreed on 3 of 4,000 mutants.

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
- **The power state** (`kchess-core/src/power.rs`, `on_battery`): it reads the OS, and no test
  can unplug the computer. Linux's rule over its power supplies and the battery's use of a
  reading are tested; the readers are excluded in `.cargo/mutants.toml`.
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
