# Releasing and app updates

## Releasing

Installers are built by `.github/workflows/release.yml`. Versions are dates with a counter,
`YEAR.MONTH.COUNTER`: the first release in October 2026 is `2026.10.0`, the next `2026.10.1`, and
the counter restarts each UTC month. There's no zero padding (`2026.10.0`, not `2026.10.00`), which
keeps the version valid semver. The release is the Cargo workspace version. Follow the
[release checklist](#release-checklist). Merge the intended changes with green CI first, then
prepare the version change:

```bash
cargo xtask release-version   # fetches origin's tags; updates Cargo.toml, Cargo.lock and WHATS_NEW.md
cargo xtask check
git add Cargo.toml Cargo.lock apps/gpui/WHATS_NEW.md
git commit -m "Release <version>"
```

Merge the reviewed version commit into `main` and wait for green CI. The version change runs CI's
packages, Snap build and Windows upgrade, which on `main` otherwise run only nightly, when
packaging may have changed since main last packaged; from a clean checkout of that commit, `cargo xtask release-verify` checks
the metadata and that its CI ran and passed every job. Then create an annotated tag derived from
its version and push only that tag, as the [release checklist](#release-checklist) shows. Never reuse or move a release tag. The version helper stops if it cannot refresh remote
tags; it does not guess using a stale local list.

The workflow builds on macOS, Windows and Linux and attaches the installers, the Snap,
the update patches, `SHA256SUMS.txt` and the update manifests to a **draft** GitHub Release; review it and press
_Publish_. A tag that isn't `vYEAR.MONTH.COUNTER` or doesn't match the workspace version fails
validation, as does a release commit outside `main` or failing repository checks. Reruns can
replace assets on an existing draft, but refuse to overwrite a published release
(`cargo xtask attach-draft-release`). _Run workflow_ in the Actions tab builds the same installers
as downloadable artifacts without creating a release, including when a tag is selected. Retry a
draft release by rerunning its original tag-push workflow.

| Platform | Files                                         |
| -------- | --------------------------------------------- |
| macOS    | `.dmg` and `.zip` for Apple Silicon (`arm64`) |
| Windows  | `.exe` installer (x64)                        |
| Linux    | `.AppImage`, `.deb` and the Snap (x64)        |

Each is made by `cargo xtask package`. The release job adds `latest-mac.yml`, `latest.yml` and
`latest-linux.yml` for Electron installations (below).

### Electron installations

Releases up to 2026.10.15 were an Electron app, whose updater reads `latest*.yml` from the newest
published release. Every release's manifests send it to 2026.11.4, the frozen stepping-stone that
still carries what an Electron install needs (its identity, the Mac zip's bridge for
`com.kchess.electron`, and the takeover of its profile); from there it updates like any KChess. The
release job's `cargo xtask electron-step` checks the step's installers against their pins and its
signed checksums, then writes manifests whose `url` is `../v2026.11.4/<name>`, which the Electron
updater resolves into that release. Electron installs depend on this step: don't change it, its
pins (`STEP` in `tooling/xtask/src/electron_step.rs`) or 2026.11.4's assets.

Without signing credentials, Windows releases are unsigned and macOS builds are ad hoc signed, so on
first launch macOS says it can't verify the developer: open **System Settings → Privacy &
Security** and choose **Open Anyway**. Windows SmartScreen shows a similar warning (**More info →
Run anyway**).

## Release checklist

For whoever releases, people and agents alike ([AGENTS.md](../AGENTS.md) keeps the rules that
must never be broken).

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
- The release's notes are `apps/gpui/WHATS_NEW.md`'s `## Unreleased` items, which
  `release-version` turns into `## <version> (<date>)` and refuses to release without. The app
  shows them once after the update (Settings → Updates → What's new keeps them), and the draft
  release and the App Store's What's New take the same text. Read them in the release commit's
  diff as a player would: only what changed for them, no internals, nothing that did not ship.
  Commit `WHATS_NEW.md` with the version.
- Release only a commit on `origin/main` whose CI is green. Include every intended file,
  including new/untracked files; explicitly stage the files instead of relying on
  `git commit -am`. Keep the working tree clean before tagging and verify CI is green
  for the intended release commit.
- Run `cargo xtask release-verify` (it needs the release commit's CI to have run and passed every
  job, including the packages, Snap and Windows upgrade) and `cargo xtask check`. Verify the
  release commit and tag before creating an annotated tag. Push **only that tag**, not `--tags` or an arbitrary
  `HEAD` branch. Never force-push, move, reuse, or delete an existing release tag.
- Tag pushes run the Release workflow and create a draft after all platform packages
  succeed. Manual `workflow_dispatch` builds produce downloadable artifacts only,
  even when the selected ref is a tag. To retry a failed draft build, rerun its original
  tag-push workflow; do not recreate the tag.
- Replacing assets is allowed only while the release is a draft. A published release
  is permanent: ship fixes with the next version and a new tag. Never bypass the
  `attach-draft-release` guard with a direct `gh release upload --clobber`.
- Supported downloads: macOS **Apple Silicon (arm64) only** (`.dmg` and `.zip`), Windows
  x64 (`.exe`), Linux x64 (`.AppImage` and `.deb`), and the Snap, as `KChess-*`. Keep their
  names, the Windows GUID, install folder, executable and uninstaller names, the `.deb` package
  and `/opt/KChess` layout: installed KChess apps update through them. Electron installations
  update to 2026.11.4 through the `latest*.yml` the release job writes; don't change that step.
  Keep `SHA256SUMS.txt` with the installers. Verify all packages and checksums before
  publishing; macOS ZIPs are required for the update feed.
- The AppImage is made with appimagetool and the AppImage runtime it embeds, both pinned by
  release and SHA-256 in `tooling/xtask/src/package/linux.rs`; packaging downloads them once
  into `target/package/`, refetches a cached copy that does not match and refuses a download
  that does not. The weekly Pins workflow (`cargo xtask pins --check`) opens one issue when
  either has a newer release; move a pin in a pull request so CI packages and tests it.

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

## Snap Store

The [`kchess` listing](https://snapcraft.io/kchess) follows this project. Release builds include a
Snap package; publishing a stable GitHub release triggers a checksum-verified upload to Snap Store
(`cargo xtask download-release-snap`) once its repository credential is configured. See
[Snap Store setup and migration](snap-store.md).

## App updates

**Settings → Updates** shows the installed version, update status, last successful check,
download progress, manual check/download/restart actions and **What's new**, every release's
notes from `apps/gpui/WHATS_NEW.md`, which also open on the first start after an update. By default,
installed builds check stable GitHub releases ten seconds after startup and every six hours,
download updates in the background, and install when the user quits. Each behaviour has its own
switch. KChess never initiates a restart while you are playing; **Restart and install** asks first.
Accounts, preferences and saved history remain in the profile folder. Development builds do not
check for updates.

The updater (`apps/gpui/src/updates.rs`) downloads the package it was installed from, checks it
against `SHA256SUMS.txt`, and refuses one not signed as the running app is (the same macOS team and
bundle identifier, or the same Windows certificate; an ad hoc Mac build accepts a valid build of
itself, ad hoc or Developer ID). It then replaces the `.app`, runs the NSIS installer silently
(`--updated /S`), replaces the AppImage, or installs the `.deb` with `pkexec dpkg -i`. A download
whose connection drops resumes from the bytes it has (`Range`), and downloads are kept from one
start to the next: a checked package is used again, and one cut short carries on. The update
check and the checksum files are tried three times after a dropped connection or a server error.

Each package also comes as **update patches** (`KChess-<version>-<platform>-<arch>.from-<old>.delta`),
from each of the three newest published releases before it, written by `cargo xtask deltas` in
each package job (`crates/kchess-delta`). The release tooling compares the old and new
releases' files by SHA-256: unchanged ones are taken from the installation, changed ones are
bsdiff patches, new ones come whole, so a release that replaces Stockfish carries it. A patch that
saves less than a quarter of the package is left out. The updater uses the patch for its own
version when there is one, rebuilds the release from the installed files, checks every file, and
stages it as the whole package's download would be (on Windows, `KChess-…-win-x64-update.exe`
inside the patch is the installer without its files, which writes the files beside it). An
installed file that is missing or changed, or any mismatch, downloads the whole package instead
(`apps/gpui/src/updates/patch.rs`). The patches are signed with everything else through
`SHA256SUMS.txt`; a failure to write them warns and costs only download size. What each update downloads, and
the plan to send only the files that changed, is in
[the updater measurements](benchmarks/2026-11-06-updater.md).

To enable Mac signing and notarization, add these GitHub Actions repository secrets:

- `MAC_CSC_LINK`: base64-encoded Developer ID Application `.p12` certificate.
- `MAC_CSC_KEY_PASSWORD`: its export password.
- `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID`: notarization credentials.

Keep the same app ID and signing identity across releases. Before uploading signed Mac packages,
the workflow verifies their code signature, signing team, Gatekeeper acceptance and stapled
notarization ticket. Never put signing credentials or a GitHub token in the app.

Publish the complete release: installers, the update patches, the manifests, `SHA256SUMS.txt`
and `SHA256SUMS.txt.sig`. Drafts and
prereleases are excluded from the feed. Do not overwrite an already published release to ship a
fix: advance `YEAR.MONTH.COUNTER`, create a new tag, and publish its draft once all platforms are
ready. macOS requires the ZIP artifacts as well as DMGs.
