# AGENTS.md — instructions for AI coding agents working in this repo

KChess is Rust: the app (`apps/gpui`, GPUI Kit), the command line (`apps/cli`), the headless core
(`crates/`) and the tooling (`tooling/xtask`, run as `cargo xtask <command>`). There is no
JavaScript, TypeScript or Node in the repository; do not add any.

## Landing changes

- Code reaches `main` through a pull request, so CI tests it on macOS, Windows and Linux before
  it lands. The local check runs only on this Mac and cannot see platform-only failures: all six
  red runs on `main` from 2026-10-29 to 2026-11-06 passed here first.
- Before opening or updating the PR, run `cargo xtask check-fast`. CI runs the full check, so
  you do not need `cargo xtask check` for a PR.
- Kill the mutants of the lines you change (docs/mutation-testing.md). CI's `mutants` job tests
  them and fails a PR when one survives; you need not run `cargo xtask mutants` before pushing.
  Write the missing test, or exclude an equivalent mutant in `.cargo/mutants.toml` with the reason.
- Open the PR and carry on. Merge it once CI is green, with auto-merge if it is enabled. Fix
  failures with new commits on the branch, not on `main`. Watch CI in the background
  (`gh run watch <id> --exit-status` with the tool's background mode); do not sit idle on it
  unless the next step needs its result.
- Documents-only changes go through a pull request too: CI skips its heavy jobs for them, so they
  merge in a minute. GitHub refuses every push straight to `main`, from anyone: its rulesets
  require the `ci-passed` check (all of `ci.yml`) and forbid force-pushing or deleting `main` and
  moving or deleting `v*` tags. Never turn a ruleset off or add a bypass to get past it.

## Releases and tags

Read [the release checklist](docs/releasing.md#release-checklist) before any release work:
versions, tagging, verification and assets.

- Creating a release means preparing a **draft**. Publish only when the user asks. A request to
  fix code, build installers or reduce size does not authorize creating or pushing a tag.
- Versions come from `cargo xtask release-version` (`YEAR.MONTH.COUNTER`, UTC month). Never pick
  one by hand from stale tags. It dates the `## Unreleased` notes in `apps/gpui/WHATS_NEW.md` and
  refuses an empty section; never invent notes to get past it: write what the release brings.
- Release only a green commit on `origin/main`, after `cargo xtask release-verify`. Push only that
  tag. Never force-push, move, reuse or delete a release tag.
- A published release is permanent: ship fixes in the next version. Replace assets only while
  the release is a draft, and never bypass the `attach-draft-release` guard.
- Keep the installer names, the Windows GUID, the install layouts, the `.deb` name and
  `/opt/KChess`, and the Snap name (`tooling/xtask/src/package`): installed apps update through them.
- Electron installs update to the frozen 2026.11.4 release through the manifests every release
  writes (`cargo xtask electron-step`, `tooling/xtask/src/electron_step.rs`); don't change that step.

## Running the app

- Check UI behaviour headless first: the end-to-end tests (`cargo test -p kchess-gpui e2e::<area>`)
  drive every page without a window. Start the real app only for what they cannot show.
- `cargo xtask app` builds and starts the app in the background on the scratch profile
  `.data/gpui-profile` with a native Stockfish (fetched into `.data/stockfish` once), prints its
  PID and logs to `.data/gpui.log`; `cargo xtask app --stop` stops it by that PID. It refuses a
  real profile. `cargo run -p kchess-gpui` also works (`tooling/macos-dev-runner.sh` signs it on
  macOS with a stable identity, so the keychain remembers it, and runs a debug build from
  `target/debug/KChess.app`, so Mission Control shows its icon); never point a development run at
  the real profile (`~/Library/Application Support/KChess`): set `KCHESS_USER_DATA_DIR` to a
  scratch folder, and `KCHESS_ENGINE_PATH` to a native Stockfish.
- `cargo xtask app --watch` stays in the foreground and rebuilds and restarts the app on every
  save under `apps/` and `crates/`, back on the same page, with the old app kept open until the
  build succeeds. Views are compiled, so there is no reload without a restart.
- `cargo xtask voice-runtime debug` stages the speech helper with libvosk for a debug build;
  `cargo xtask app` does it when the helper is missing or older than the build.
- `cargo run -p kchess-cli -- --help` is the command line; it shares the profile's core with a
  running app through the core service (`docs/shared-core.md`).
- Driving the app with synthetic input: start it by PID and check that PID is frontmost before
  every click or keystroke; stop it with `kill <pid>`, never by binary path.
- `screencapture` triggers a macOS screen-recording permission prompt for the host on first use
  — prefer AppleScript process checks and reserve screenshots for visual layout verification.
- Do not edit generated output: `target/`, `apps/gpui/dist/`. App icons live in
  `apps/gpui/build/` and are generated by `cargo xtask app-icon` — change
  `tooling/xtask/src/app_icon.rs` and re-run it rather than editing the images. Only original or clearly open-licensed artwork may go
  in this repo (see `THIRD_PARTY_NOTICES.md`).

## UI copy and information density

- Keep the interface quiet: show only text needed to identify an action, make a decision,
  understand the current state, or recover from a problem. Apply this across every page,
  panel, dialog, empty state, and settings section when creating or changing UI.
- Do not add permanent boilerplate such as “Internet required”, “No account needed”,
  “Works offline”, “Nothing is sent”, or explanations of obvious buttons and settings.
  Show connection or account guidance only when it actually blocks the current action.
- Use one clear label per action. Avoid repeating a heading in a description and again
  in the button. Prefer useful empty-state actions over paragraphs of onboarding copy.
- Keep real state and consequences: game status, scores, progress, errors, unsaved work,
  destructive-action warnings, rated-result effects, privacy choices, and download sizes.
  Explain non-obvious behavior briefly where the user makes that choice; put optional
  instructions in tooltips or expandable help instead of permanent body text.
- Settings labels and choices should stand on their own. Remove hints that merely
  paraphrase them; retain concise hints for non-obvious effects or tradeoffs.
- Remove unnecessary copy from the source rather than hiding it. Preserve accessible
  names, roles, labels, and live status/error announcements. Update affected
  tests to verify actions and state rather than require discarded explanatory copy.
- Never send people to a website to finish something KChess can do itself.

## What's new

`apps/gpui/WHATS_NEW.md` is what people see after an update, in the app, on the GitHub release
and in the App Store. Its readers play chess; they are not developers.

- A change a player would notice adds one line under `## Unreleased`, in the same pull request:
  `- **Short name**: what they can now do, or what no longer goes wrong.`
- Write as you would tell a friend who plays chess: everyday words, one or two short sentences,
  "you" and what it does for them. Name things as the app's own screens and buttons name them.
- Never mention code, crates, modules, files, APIs, PRs, refactors, tests, CI, tooling, builds,
  benchmarks, audits or formats a player has no reason to know. Explain a chess term only when
  the feature is that term (Crazyhouse: "captured pieces join your side").
  - Not: "Spaced repetition for the repertoire trainer (SM-2 scheduling)."
  - But: "**Smarter opening practice**: the opening trainer brings back the moves you got wrong
    until you remember them."
  - Not: "Fix memory leaks found by retention benchmarks."
  - But: "**Fixes**: KChess uses less memory over long sessions."
- Only what is in the build and works. Fold small fixes into one `**Fixes**` line. Changes with
  nothing a player would see (internal, tests, docs, tooling) add nothing.
- Extend an existing line when a later change grows the same feature. Never edit a dated
  section: a published release's notes are permanent too.

## Reuse existing UI and behavior

- Before building a new UI or interaction, inspect existing views and equivalent
  screens. Reuse their components, widgets and behavior wherever possible.
- If an existing implementation is page-local, extract it into a shared component and
  migrate both uses. Improve the shared component to support the new use case before
  introducing a separate implementation. Keep related surfaces consistent in density,
  accessibility, keyboard behavior, previews, and settings.
- Create a separate implementation only when the existing component cannot reasonably
  fit the new use case; explain the concrete constraint. Do not duplicate functionality
  merely because it appears on a different page.

## Architecture and regression ownership

- The headless core (`crates/kchess-core`) owns credentials, networking, privileged files,
  documents and engine processes; it never depends on a frontend. Frontends (the app, the CLI)
  call it by `CoreApi` method name and subscribe to its events, usually through the profile's
  shared core service (`crates/kchess-service`, `docs/shared-core.md`); host capabilities they
  answer go through `host:request`. A new method belongs in `kchess-contracts` (`core_api.rs`),
  the facade and its validator (`kchess-domain/src/misc/contracts.rs`).
- Call the core with a typed call (`kchess_contracts::calls`, `AppCore::request`) when its answer
  has a contract type: one line in `calls!` names the method, its arguments and its answer, and
  the core's tests check it against the facade. Move a `call("…")` to one when you edit it.
- Frontends import `CoreError`, `ErrorKind` and `Level` from `kchess-contracts`. Only the files
  that start, configure or host the core name `kchess_core`: xtask's `boundaries` test holds the
  list (`tooling/xtask/src/boundaries.rs`, run by CI). Add to it only for host wiring, never to
  reach a service. The app's own HTTP is the updater (`updates.rs`, which replaces the app) and
  Lichess's pictures (`remote_image.rs`), neither with a login; any other download (the engine,
  the puzzle database, the voice model) is a core method with a progress event.
- Pure chess and training rules live in `crates/kchess-domain` (`api.rs` is their one entry);
  game sessions as frontends drive them in `crates/kchess-games`. View preferences (tabs, sort
  orders, layout) are the core's (`viewPreferences`), so every frontend opens a page as it was
  left.
- `tests/rules_corpus.rs` (kchess-domain) pins every rules method to answers recorded from the
  former TypeScript suites; rewrite it (`KCHESS_WRITE_CORPUS=1`) only for an intended behaviour
  change.
- `UciController` (`crates/kchess-core/src/engine/uci.rs`) owns UCI deadlines and pipes; acquire
  the shared engine scheduler (`EngineContext::lease`, `engine/mod.rs`) before configuring search
  threads and release it on every path. Cancellation must reach the search, not merely discard
  its result. Infinite analysis has no runtime deadline, but startup/readiness/stop remain bounded.
- Preserve root FEN plus move history for repetition context. Analysis caches distinguish
  completed/interrupted/failed output and engine/context/client identity. Do not mark
  interrupted output as complete.
- The Rust puzzle service (`crates/kchess-core/src/puzzles`) is the only owner of `puzzles.db`.
  Failed or cancelled replacements retain the previous transaction and metadata; migrations are
  append-only.
- The core's library (`crates/kchess-core/src/store/library.rs`) owns studies, played games,
  mistake drills, unfinished sessions, joined tournaments and repertoire notes as bounded,
  versioned documents with semantic move validation. PGN edits must preserve headers, results,
  variations, comments and NAGs.
- Lichess stream events are authoritative. Connection recovery must retain the playing account
  and reject stale epochs. Never retry an ambiguous move mutation automatically, and never expose
  engine/review/explorer assistance during a live human game.
- Use deterministic local HTTP/stream fixtures. The app's end-to-end tests (`apps/gpui/src/e2e`)
  drive the whole app headless.

## Executable guardrails

- Read GUARDRAILS.md for the verification ladder.
- Test in tiers. While editing, run `cargo xtask check-fast` (format, lints and the tests of the
  packages changed since main and of every package depending on them). Run tests with `cargo nextest
  run` (`cargo install cargo-nextest`; `.config/nextest.toml`): it runs test binaries in parallel
  where `cargo test` runs them one at a time. The app's tests are the exception: one binary whose
  tests each start GPUI, faster under `cargo test -p kchess-gpui`. CI runs the full `cargo xtask check` on the pull
  request. Packaging changes
  also need `cargo xtask package` on the platform they touch; CI then packages, builds the Snap and
  runs the Windows upgrade (`ci.yml` `changes`).
- Run anything that may take over a minute (`cargo xtask check`, the app's tests, packaging) in
  the background with its output in a file, and read the result when it finishes. Never wait in
  a loop (`sleep`, `until grep …; do sleep`, `while pgrep`): the tool reports when a background
  command ends, and `gh run watch` in the background reports when a CI run ends.
- The development shell is zsh on macOS. Quote globs and `grep --include` patterns (an unmatched
  glob is an error in zsh), and do not call `timeout`, which macOS lacks: use the tool's own
  timeout or run the command in the background.
- Several agent sessions often work in this repository at once, each in its own worktree. Never
  use `git stash` (one stash list serves every worktree, so a pop can take another session's
  work); keep work in progress as commits on your own branch, and never commit in a checkout or
  on a branch another session is using.
- Fix why something breaks, not only its symptom. Before closing, reverting, skipping or ignoring
  a failing check, broken pull request (Dependabot included) or bad config, find what made it fail
  and whether a small change stops it for good; do that change, and offer the stopgap only as a
  fallback. Dependabot once moved every `taiki-e/install-action` pin to one tool's tag commit, so
  every step installed that tool; the fix was one pinned release with a `tool:` input.
- Bug fixes must retain a regression for the concrete failure sequence. When generated
  tests expose a failure, preserve its replay seed/path and a focused reproducer.
- Do not lower the coverage floors (`tooling/xtask/src/check.rs`) without a measurement that
  explains the drop.
