# Releasing and app updates

## Releasing

Installers are built by `.github/workflows/release.yml`. Versions are dates with a counter,
`YEAR.MONTH.COUNTER`: the first release in October 2026 is `2026.10.0`, the next `2026.10.1`, and
the counter restarts each UTC month. There's no zero padding (`2026.10.0`, not `2026.10.00`), which keeps
the version valid semver. Follow the release checklist in [AGENTS.md](../AGENTS.md).
Merge the intended changes with green CI first, then prepare the version change:

```bash
pnpm run release:version   # fetches origin's tags; updates package.json; dependencies stay locked
pnpm run release:verify
pnpm run check
git diff -- package.json pnpm-lock.yaml
git add package.json pnpm-lock.yaml
git commit -m "Release $(node -p "require('./package.json').version")"
```

Merge the reviewed version commit into `main` and wait for green CI. From a clean checkout
of that commit, create an annotated tag derived from its package version and push only
that tag, as shown in `AGENTS.md`. Never reuse or move a release tag. The version helper
stops if it cannot refresh remote tags; it does not guess using a stale local list.

The workflow builds on macOS, Windows and Linux and attaches the installers, plus `SHA256SUMS.txt`,
and updater manifests/blockmaps to a **draft** GitHub Release; review it and press _Publish_. A tag that isn't `vYEAR.MONTH.COUNTER`
or doesn't match `package.json` fails validation, as does
a release commit outside `main` or failing repository checks. Reruns can replace assets
on an existing draft, but refuse to overwrite a published release. Creating a release
prepares a draft; publishing it is a separate action.
_Run workflow_ in the Actions tab builds the same installers as downloadable artifacts
without creating a release, including when a tag is selected. Retry a draft release by
rerunning its original tag-push workflow.

| Platform | Files                                         |
| -------- | --------------------------------------------- |
| macOS    | `.dmg` and `.zip` for Apple Silicon (`arm64`) |
| Windows  | `.exe` installer (x64)                        |
| Linux    | `.AppImage` and `.deb` (x64)                  |

The same release carries the GPUI preview (`KChess-Preview-*`, built by the `gpui` job with
`tooling/gpui-package.mjs`) in the same formats. It installs beside the Electron app under its own
name, install folder, NSIS GUID and `.deb` package, and writes no `latest*.yml`, so Electron's
updater never reads it as its own. On macOS the `gpui` job signs and notarizes it with the same
Developer ID secrets as the Electron app, and verifies it as the `package` job does.

### Switching KChess to the GPUI app

`KCHESS_RELEASE_APP` at the top of `release.yml` names the app that ships as KChess. With `gpui`,
the `gpui` job also builds `gpui-package.mjs --final`: the GPUI app under Electron's app ID,
product name, NSIS GUID and `.deb` package, with the `latest*.yml` manifests, so Electron's updater
installs it over the Electron app and its profile carries on. The Electron job then builds only the
Snap, which stays the Electron app; the preview keeps shipping so its installations keep updating.
The switch is a reviewed commit like any other, released with the next tag; a published release
cannot be taken back, so before flipping it:

- run **Windows upgrade** from the Actions tab: it upgrades published Electron installs (per user,
  custom folder, all users) with the GPUI installer as well as Electron's, and checks Apps &
  features, shortcuts, the old copy and user data;
- on a Mac with a Developer ID signed Electron install, check that its updater installs a signed
  `--final` build (Squirrel.Mac accepts only the same team and bundle identifier);
- check that an AppImage and a `.deb` Electron install update to the GPUI app and keep the profile.

Without signing credentials, Windows releases are unsigned and macOS builds are ad-hoc signed (`tooling/release-config.cjs`), so on
first launch macOS says it can't verify the developer: open **System Settings → Privacy & Security**
and choose **Open Anyway**. Windows SmartScreen shows a similar warning (**More info → Run anyway**).

## Snap Store

The existing [`kchess` listing](https://snapcraft.io/kchess) now follows this project.
Release builds include a Snap package; publishing a stable GitHub release triggers a
checksum-verified upload to Snap Store once its repository credential is configured.
See [Snap Store setup and migration](snap-store.md).

## App updates

**Settings → Updates** shows the installed version, update status, last successful check,
download progress, manual check/download/restart actions and a link to release notes.
By default, installed builds check stable GitHub releases ten seconds after startup and every
six hours, download updates in the background, and install when the user quits. Each behaviour
has its own switch. KChess never initiates a restart while you are playing; **Restart and install**
asks first. A normal quit with an update ready installs it without reopening the app.
Accounts, preferences and saved history remain in the existing user-data directory.
Development builds do not check for updates; unsupported unpacked installations cannot update themselves.

Windows NSIS, Linux AppImage and Debian installations support automatic installation
(Debian may require an operating-system authorization prompt). Ad-hoc signed Mac releases
check for updates and link to a manual download. [macOS automatic installation requires
Developer ID signing](https://www.electron.build/v26/docs/features/auto-update/).

The GPUI app has its own updater (`apps/gpui/src/updates.rs`) on the same feed, with the same
switches. It downloads the package it was installed from, checks it against `SHA256SUMS.txt`, and
refuses one not signed as the running app is (the same macOS team and bundle identifier, or the
same Windows certificate; an ad hoc Mac build accepts a valid build of itself, ad hoc or Developer ID). It then
replaces the `.app`, runs the NSIS installer silently (`--updated /S`, as Electron's updater does),
replaces the AppImage, or installs the `.deb` with `pkexec dpkg -i`. `gpui-package.mjs --final`
builds the app under Electron's identity with the `latest*.yml` manifests, so that at the switch
Electron's own updater installs it over the Electron app.

To enable Mac automatic installation, add these GitHub Actions repository secrets:

- `MAC_CSC_LINK`: base64-encoded Developer ID Application `.p12` certificate.
- `MAC_CSC_KEY_PASSWORD`: its export password.
- `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID`: notarization credentials.

The release config imports the certificate, requires signing and notarization, and marks only
those Mac builds as able to install updates. Keep the same app ID and signing identity across
releases. Before uploading signed Mac packages, the workflow verifies their code signature,
signing team, Gatekeeper acceptance and stapled notarization ticket.
Never put signing credentials or a GitHub token in the app. Windows signing can be
added separately without changing the update feed.

Publish the complete release: installers, `latest.yml` (Windows), `latest-mac.yml` (Mac),
`latest-linux.yml` (Linux), and `.blockmap` files. The workflow verifies and attaches the manifests;
electron-builder generates the packaged `app-update.yml` from the fixed public GitHub provider.
Drafts and prereleases are excluded. Do not overwrite an already published release to ship a
fix: advance `YEAR.MONTH.COUNTER`, create a new tag, and publish its draft once all platforms
are ready. macOS requires the ZIP artifacts as well as DMGs.

**Bootstrap:** releases shipped before the updater was added cannot acquire it automatically.
Users must install the first updater-enabled release manually; subsequent releases use this flow.
An ad-hoc Mac installation also needs a manual upgrade to the first Developer ID signed release.
Before publishing broadly, verify a real version A → B update on installed copies of each
supported platform, including restart, normal quit, opt-out, offline retry and data preservation.
Mocked tests and local unsigned builds cannot verify signing or the published feed end to end.
