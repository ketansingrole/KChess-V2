# AGENTS.md — instructions for AI coding agents working in this repo

KChess is Rust: the app (`apps/gpui`, GPUI Kit), the command line (`apps/cli`), the headless core
(`crates/`) and the tooling (`tooling/xtask`, run as `cargo xtask <command>`). There is no
JavaScript, TypeScript or Node in the repository; do not add any.

## Releases and tags

Creating a release means preparing a **draft**. Publish it only when the user asks to
publish. A request to fix code, build installers, or reduce size does not authorize
creating/pushing a tag or publishing a release.

- Versions are `YEAR.MONTH.COUNTER`, using the **UTC release month**. Start a new month
  at `0`; otherwise use one more than the highest existing tag's counter for that month.
  No zero padding: workspace version `2026.10.1`, Git tag `v2026.10.1`.
  Do not use ordinary `v1.2.3`, date/day tags, prerelease suffixes, or guess a counter.
- Use `cargo xtask release-version`. It must successfully fetch tags from `origin` first.
  If fetching fails, restore access and retry; do not select a version from stale tags.
  Commit `Cargo.toml` and `Cargo.lock`; the lockfile must match the workspace version.
- Changes may land on `main` directly or through a pull request; a PR is optional. Before
  pushing to `main`, run `cargo xtask check` locally (the `.githooks/pre-push` hook, installed by
  `cargo xtask install-hooks`, runs it and refuses a dirty tree; skip it only on purpose with
  `KCHESS_SKIP_PRE_PUSH=1`), and after pushing, confirm CI is green and
  fix any failure with a follow-up commit (never force-push `main`).
- Release only a commit on `origin/main` whose CI is green. Include every intended file,
  including new/untracked files; explicitly stage the files instead of relying on
  `git commit -am`. Keep the working tree clean before tagging and verify CI is green
  for the intended release commit.
- Run `cargo xtask release-verify` and `cargo xtask check`. Verify the release commit and tag
  before creating an annotated tag. Push **only that tag**, not `--tags` or an arbitrary
  `HEAD` branch. Never force-push, move, reuse, or delete an existing release tag.
- Tag pushes run the Release workflow and create a draft after all platform packages
  succeed. Manual `workflow_dispatch` builds produce downloadable artifacts only,
  even when the selected ref is a tag. To retry a failed draft build, rerun its original
  tag-push workflow; do not recreate the tag.
- Replacing assets is allowed only while the release is a draft. A published release
  is permanent: ship fixes with the next version and a new tag. Never bypass the
  `attach-draft-release` guard with a direct `gh release upload --clobber`.
- Supported downloads: macOS **Apple Silicon (arm64) only** (`.dmg` and `.zip`), Windows
  x64 (`.exe`), Linux x64 (`.AppImage` and `.deb`), and the Snap, as `KChess-*` (the final
  identity, which writes `latest*.yml`) and `KChess-Preview-*` (which must never ship
  `latest*.yml`). Every Electron installation ever released updates itself to the final build
  through those manifests: keep their names, the Windows GUID, install folder, executable and
  uninstaller names, and the `.deb` package and `/opt/KChess` layout (`tooling/xtask/src/package`).
  Keep `SHA256SUMS.txt` with the installers. Verify all packages and checksums before
  publishing; macOS ZIPs are required for the update feed.

Before an authorized tag push, derive its name from the committed workspace version,
confirm the named commit is the intended release, and inspect the exact local tag:

```bash
release_tag="v$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
release_commit="$(git rev-parse HEAD)"
git status --short                          # must be empty
git merge-base --is-ancestor "$release_commit" origin/main  # must succeed
git tag --list "$release_tag"               # must be empty for a new release
git tag -a "$release_tag" -m "KChess ${release_tag#v}" "$release_commit"
git show --no-patch "$release_tag"           # verify version and target commit
git push origin "refs/tags/$release_tag"
```

## Running the app

- `cargo run -p kchess-gpui` builds and starts the app (`tooling/macos-dev-runner.sh` signs it
  on macOS with a stable identity, so the keychain remembers it). Never point a development run
  at the real profile (`~/Library/Application Support/KChess`): set `KCHESS_USER_DATA_DIR` to a
  scratch folder, and `KCHESS_ENGINE_PATH` to a native Stockfish.
- `cargo xtask voice-runtime debug` stages the speech helper with libvosk for a debug build.
- `cargo run -p kchess-cli -- --help` is the command line; it shares the profile's core with a
  running app through the core service (`docs/shared-core.md`).
- Driving the app with synthetic input: start it by PID and check that PID is frontmost before
  every click or keystroke; stop it with `kill <pid>`, never by binary path.
- `screencapture` triggers a macOS screen-recording permission prompt for the host on first use
  — prefer AppleScript process checks and reserve screenshots for visual layout verification.
- Do not edit generated output: `target/`, `apps/gpui/dist/`. App icons live in
  `apps/gpui/build/` and are generated by `python3 tooling/make-icons.py` — change the script and
  re-run it rather than editing the images. Only original or clearly open-licensed artwork may go
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
- Pure chess and training rules live in `crates/kchess-domain` (`api.rs` is their one entry);
  game sessions as frontends drive them in `crates/kchess-games`. View preferences (tabs, sort
  orders, layout) are the core's (`viewPreferences`), so every frontend opens a page as it was
  left.
- `tests/rules_corpus.rs` (kchess-domain) pins every rules method to answers recorded from the
  former TypeScript suites; rewrite it (`KCHESS_WRITE_CORPUS=1`) only for an intended behaviour
  change.
- What the Electron app left in a profile (its `localStorage` documents and preferences, logins
  its `safeStorage` sealed) is taken over by the core itself (`crates/kchess-electron-legacy`);
  keep that working for profiles from any Electron release.
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
  packages changed since main). Before pushing to `main`, run the full `cargo xtask check` (the
  pre-push hook enforces it). Packaging changes also need `cargo xtask package` on the platform
  they touch, and the Windows upgrade workflow for installer changes.
- Bug fixes must retain a regression for the concrete failure sequence. When generated
  tests expose a failure, preserve its replay seed/path and a focused reproducer.
- Do not lower the coverage floors (`tooling/xtask/src/check.rs`) without a measurement that
  explains the drop.
