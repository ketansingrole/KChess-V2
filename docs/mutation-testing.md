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
survives or times out. Run it before pushing; `--base <ref>` diffs against another commit, and
arguments after `--` go to cargo-mutants (`-- -j 4`, `-- -p kchess-domain`). Mutants that were
already missed in untouched code are the weekly run's, so a change never has to pay off the
whole backlog at once.

## What to do with a missed mutant

1. Write the test it shows is missing, at the level that observes the behaviour: a unit test, a
   CLI or service test over a scratch profile, or a local fixture (the CLI's tests serve a
   stand-in Lichess and use the core's scripted engine, `kchess-fake-uci`).
2. When the code is hard to observe, make it observable rather than test around it: the voice
   helper binds libvosk from any symbol source, so its tests bind in-process fakes; the CLI's
   configuration takes the environment as a parameter, and its host answers take the browser
   opener.
3. When no test can tell the mutant apart because the program behaves the same (an *equivalent*
   mutant), exclude it in `.cargo/mutants.toml` with the reason. Never exclude one because a
   test is missing. Prefer exclusions by function name to line numbers, which move.

## Weekly runs

`.github/workflows/mutants.yml` runs a package in eight shards every Saturday (the rules) and on
demand for any package. The rules and the core take hours on one machine: about 20 seconds of
CPU per mutant for the rules' 7,000, more for the core's 6,300 and the app's 12,000, whose every
mutant rebuilds the app.

## Mutants no test here can kill

These stay in the reports on purpose; each is listed with the reason.

- **Windows-only code** (`kchess-service`'s named pipes and owner checks, `launch`'s creation
  flags): not compiled on Linux or macOS, so a mutant there builds and passes on them. They can
  only be caught by a run on Windows. Likewise the CLI's import of the old Mac database
  (`config_from`'s `legacy_database_path`) is caught only on macOS, where its test checks it.
- **Stack sizes** (`kchess-service`'s and the CLI's 8 MiB core threads): a smaller stack changes
  nothing until a call overflows it, and no deterministic test overflows a 1 MiB stack on
  Linux or macOS (the case it guards, an embedded sign-in in a Windows debug build, needs
  Windows).
- **The service's slow-reader cleanup** (`server.rs`, `handle`'s last `if`): whether a writer
  stuck on a client that stopped reading is cancelled now or left until the client reads again,
  the client receives the same frames and the same end.
- **Per-window `suspend`** (`server.rs`): the difference from the core-wide `suspend` shows
  only in an engine search or a puzzle download of another window.
- **The CLI's own list of saved games** (`play.rs`, `written`): it only feeds the Game history
  rules within one run, which find an entry by its id with or without the duplicates a mutant
  leaves; the saved library is the same.
- **How the CLI's `review` recognises its review** (`review.rs`, `run`'s `ours`): by its moves
  and its start position; one run asks for one review, so no other review's update arrives to
  be mistaken for it.
- **The CLI's `close` and `open_browser`**: the process exits right after `close`, with the
  core's writes already on disk; `open_browser` starts the desktop's opener, which a test must
  not do.
