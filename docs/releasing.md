# Releasing and app updates

## Releasing

Installers are built by `.github/workflows/release.yml`. Versions are dates with a counter,
`YEAR.MONTH.COUNTER`: the first release in October 2026 is `2026.10.0`, the next `2026.10.1`, and
the counter restarts each UTC month. There's no zero padding (`2026.10.0`, not `2026.10.00`), which
keeps the version valid semver. The release is the Cargo workspace version. Follow the release
checklist in [AGENTS.md](../AGENTS.md). Merge the intended changes with green CI first, then prepare
the version change:

```bash
cargo xtask release-version   # fetches origin's tags; updates Cargo.toml and Cargo.lock
cargo xtask release-verify
cargo xtask check
git add Cargo.toml Cargo.lock
git commit -m "Release <version>"
```

Merge the reviewed version commit into `main` and wait for green CI. From a clean checkout of that
commit, create an annotated tag derived from its version and push only that tag, as shown in
`AGENTS.md`. Never reuse or move a release tag. The version helper stops if it cannot refresh remote
tags; it does not guess using a stale local list.

The workflow builds on macOS, Windows and Linux and attaches the installers, the Snap,
`SHA256SUMS.txt` and the update manifests to a **draft** GitHub Release; review it and press
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

Each is made by `cargo xtask package` in two identities: `KChess-*` (`--final`), the app every
KChess installation runs, with the `latest*.yml` manifests; and `KChess-Preview-*`, which installs
beside it under its own name, folder, NSIS GUID and `.deb` package and writes no manifests, so
preview installations keep updating as themselves.

### The move from the Electron app

Releases up to 2026.10.15 were an Electron app. Its updater (electron-updater) reads the same
GitHub feed through `latest-mac.yml`, `latest.yml` and `latest-linux.yml`, so the first Rust release
reaches every Electron installation as an ordinary update, from any earlier version:

- **Windows**: the installer keeps the Electron app's identity (per-user, GUID
  `6bb66f38-a1a5-5ec8-b24f-c899466885ca`, `%LOCALAPPDATA%\Programs\KChess` or the existing folder,
  `KChess.exe`, `Uninstall KChess.exe`, the Start menu and desktop shortcuts), runs the Electron
  installation's own uninstaller in place, keeping user data, and removes an all-users copy from
  releases up to 2026.10.11. The **Windows upgrade** workflow checks per-user, custom-folder,
  all-users and fresh installs.
- **macOS**: Squirrel.Mac installs the `.zip` when it carries the same bundle identifier
  (`com.kchess.app`) and Developer ID team; ad hoc signed installations link to a manual download.
- **Linux**: the AppImage replaces the running one; the `.deb` (package `kchess`, `/opt/KChess`)
  upgrades the Electron package in place and dpkg removes its files. The Snap keeps the store name
  `kchess` and its plugs.
- **The profile**: the app opens the same folder. The core takes over what the Electron renderer
  kept in `localStorage` (studies, games and sessions from before the core kept them, and page
  settings) and opens Lichess logins sealed with Electron's `safeStorage`, sealing them again with
  the profile's own key (`crates/kchess-electron-legacy`).

Without signing credentials, Windows releases are unsigned and macOS builds are ad hoc signed, so on
first launch macOS says it can't verify the developer: open **System Settings → Privacy &
Security** and choose **Open Anyway**. Windows SmartScreen shows a similar warning (**More info →
Run anyway**).

## Snap Store

The [`kchess` listing](https://snapcraft.io/kchess) follows this project. Release builds include a
Snap package; publishing a stable GitHub release triggers a checksum-verified upload to Snap Store
(`cargo xtask download-release-snap`) once its repository credential is configured. See
[Snap Store setup and migration](snap-store.md).

## App updates

**Settings → Updates** shows the installed version, update status, last successful check,
download progress, manual check/download/restart actions and a link to release notes. By default,
installed builds check stable GitHub releases ten seconds after startup and every six hours,
download updates in the background, and install when the user quits. Each behaviour has its own
switch. KChess never initiates a restart while you are playing; **Restart and install** asks first.
Accounts, preferences and saved history remain in the profile folder. Development builds do not
check for updates.

The updater (`apps/gpui/src/updates.rs`) downloads the package it was installed from, checks it
against `SHA256SUMS.txt`, and refuses one not signed as the running app is (the same macOS team and
bundle identifier, or the same Windows certificate; an ad hoc Mac build accepts a valid build of
itself, ad hoc or Developer ID). It then replaces the `.app`, runs the NSIS installer silently
(`--updated /S`), replaces the AppImage, or installs the `.deb` with `pkexec dpkg -i`.

To enable Mac signing and notarization, add these GitHub Actions repository secrets:

- `MAC_CSC_LINK`: base64-encoded Developer ID Application `.p12` certificate.
- `MAC_CSC_KEY_PASSWORD`: its export password.
- `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID`: notarization credentials.

Keep the same app ID and signing identity across releases. Before uploading signed Mac packages,
the workflow verifies their code signature, signing team, Gatekeeper acceptance and stapled
notarization ticket. Never put signing credentials or a GitHub token in the app.

Publish the complete release: installers, the manifests and `SHA256SUMS.txt`. Drafts and
prereleases are excluded from the feed. Do not overwrite an already published release to ship a
fix: advance `YEAR.MONTH.COUNTER`, create a new tag, and publish its draft once all platforms are
ready. macOS requires the ZIP artifacts as well as DMGs.
