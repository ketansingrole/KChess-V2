# Snap Store

The existing `kchess` Snap Store identity belongs to Ketan Singrole. Its new source is
[ketansingrole/KChess-V2](https://github.com/ketansingrole/KChess-V2), replacing the old
`ketansingrole/KChess` project. Keep the registered snap name so installed users receive updates.

## Release flow

The Release workflow builds an amd64, strict-confinement core24 Snap using the same desktop
payload as the AppImage and Debian packages. An isolated LXD build supplies the Ubuntu 24.04
runtime, matching the Linux native module build host. The Snap is attached to the draft with
`SHA256SUMS.txt`. Manual Release runs only produce artifacts.

After reviewing and publishing a stable GitHub release, Publish Snap downloads that release's
versioned `.snap`, verifies its SHA-256 against the published checksums, and uploads it to
`latest/stable`. Drafts and prereleases do not publish to the store. For upload failures,
rerun the failed Publish Snap job; do not move the tag or replace published assets.

Snap Store manages installed Snap updates. KChess disables its GitHub updater inside a Snap.
The sandbox needs network access, a localhost listener for Lichess OAuth, desktop/GPU/audio
interfaces, and home access for PGNs. Microphone and password-manager interfaces may need
manual connection or Store approval:

```sh
sudo snap connect kchess:audio-record
sudo snap connect kchess:password-manager-service
```

Test on a Linux desktop before the first stable migration: launch the installed Snap, play
Stockfish, log in to Lichess, restart with saved credentials, import/export a PGN, and test
voice input. Existing users should back up important PGNs before the first V2 update; this
project does not promise migration of the legacy 2020 application's data format.

## Electron Builder desktop-entry patch

`patches/app-builder-lib@26.17.0.patch` removes the core24 generator's premature
`desktop: meta/gui/kchess.desktop` field. Snapcraft discovers the desktop file that
Electron Builder writes to `snap/gui` and creates `meta/gui` while packing. Without
this patch, Snapcraft 9 fails after priming because that output path does not exist
yet. The installed-generator regression test covers the generated descriptor. Recheck
this patch when upgrading Electron Builder and remove it once upstream handles the path.

## One-time publishing credential

On a trusted Linux machine with Snapcraft, export a credential scoped to this package and
stable channel, then store it directly as a GitHub Actions secret (never commit it or paste it
into chat):

```sh
snapcraft export-login --snaps=kchess --channels=latest/stable \
  --acls=package_access,package_push,package_release snap-store.credentials
gh secret set SNAPCRAFT_STORE_CREDENTIALS --repo ketansingrole/KChess-V2 < snap-store.credentials
rm snap-store.credentials
```

Set an expiry suitable for your maintenance policy using `--expires` and renew before expiry.
See [Snapcraft authentication](https://snapcraft.io/docs/snapcraft-authentication/).
Do not connect Snapcraft's automatic source builds to the old repository; GitHub release
publication is the only intended stable publishing trigger.

## Credential setup from macOS with Colima

The existing Colima VM can run Snapcraft; a separate Linux machine is unnecessary.
Install it once inside the VM:

```sh
colima ssh -- sudo sh -c 'apt-get update && apt-get install -y snapd && systemctl enable --now snapd.socket && snap wait system seed.loaded && snap install snapcraft --classic'
```

Run the export interactively in your Mac Terminal, complete Ubuntu One authentication,
and send the file directly to GitHub without printing its contents:

```sh
colima ssh -- sh -c 'umask 077; snapcraft export-login --snaps=kchess --channels=latest/stable --acls=package_access,package_push,package_release /tmp/kchess-store.credentials'
colima ssh -- cat /tmp/kchess-store.credentials | gh secret set SNAPCRAFT_STORE_CREDENTIALS --repo ketansingrole/KChess-V2
# Remove only after gh confirms the secret was stored successfully.
colima ssh -- rm /tmp/kchess-store.credentials
```

## Listing

Maintain the title, summary, description, icon, current screenshots, GPL-3.0-or-later license,
source URL and issue URL at [the listing dashboard](https://snapcraft.io/kchess/listing).
Store metadata is managed separately from package uploads; do not overwrite curated store
copy automatically on each release.
