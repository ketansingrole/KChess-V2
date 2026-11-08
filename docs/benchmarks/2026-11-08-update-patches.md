# Update patches: download size and time, before and after (2026-11-08)

What an update costs on each platform before and after update patches (`crates/kchess-delta`,
`cargo xtask deltas`, `apps/gpui/src/updates/patch.rs`), measured on the published packages of
2026.11.2, 2026.11.4 and 2026.11.5. It works through items 3 and 4 of the
[updater plan](2026-11-06-updater.md#plan).

## How it was measured

- **Packages**: the release assets as published on GitHub.
- **Patches**: `cargo xtask deltas --from <old package> <new package>`, the command the release
  workflow runs, for the .deb, the AppImage and the macOS zip. On Windows that command installs
  each installer silently to read its files, which needs Windows. The Windows figures instead come
  from the installers' own compressed data, unpacked with Python's `lzma` module and paired file
  by file, with 200 KB allowed for the update installer (the NSIS stub is under 100 KB).
- **Rebuilds**: `Delta::rebuild` from the old release's files, checked file by file against the
  new package's (every file identical; the AppImage byte for byte). Measured on a 4-core Linux
  x86-64 machine; the rebuild is the same code on every platform and mostly reads and writes the
  disk. The .deb's extra step, writing the rebuilt files into a `.deb`, took 0.2 s more.
- **Times**: the download at a steady 10, 50 and 100 Mbit/s, plus the rebuild. Putting the update
  in place (on quit, or **Restart and install**) is the same step as before and is left out.

## Fresh downloads

What a new user downloads. Only the .deb changes: its program files are now xz rather than gzip
(`tooling/xtask/src/package/linux.rs`), size measured on 2026.11.5's files.

| Platform | Package                        | Before   | After        |
| -------- | ------------------------------ | -------: | -----------: |
| Linux    | `KChess-…-linux-amd64.deb`     | 135.6 MB | **105.2 MB** |
| Linux    | `KChess-…-linux-x86_64.AppImage` | 130.5 MB | 130.5 MB   |
| macOS    | `KChess-…-mac-arm64.dmg`       | 120.3 MB | 120.3 MB     |
| Windows  | `KChess-…-win-x64.exe`         | 112.3 MB | 112.3 MB     |

The floor is Stockfish: 103 MB with its networks built in, which compress to about 80 MB at best.
The AppImage keeps zstd, which reads fast at every start; xz would save about a fifth and slow
every launch. The Windows installer is already solid LZMA. The macOS zip has to stay a zip that
`ditto` unpacks.

## Update downloads

What an installed KChess downloads to update. Before, every update was the whole package.

| Platform        | Before (whole package) | 2026.11.4 → 2026.11.5 | 2026.11.2 → 2026.11.5 |
| --------------- | ---------------------: | --------------------: | --------------------: |
| Linux .deb      | 135.6 MB               | **6.6 MB** (−95%)     | **16.4 MB** (−88%)    |
| Linux AppImage  | 130.5 MB               | **41.7 MB** (−68%)    | **42.1 MB** (−68%)    |
| macOS           | 119.3 MB               | **4.6 MB** (−96%)     | **13.2 MB** (−89%)    |
| Windows         | 112.3 MB               | **6.3 MB** (−94%)     | **16.0 MB** (−86%)    |

- **2026.11.4 → 2026.11.5**: the app, the core service and the command line changed, with the
  licences text. Stockfish, Fairy-Stockfish and the voice runtime did not, and are taken from the
  installation (each checked by its SHA-256).
- **2026.11.2 → 2026.11.5**: three releases back. Fairy-Stockfish, new in 2026.11.3, comes whole
  (about 1 MB on Linux, 4 MB on Windows). Each release publishes patches from the three newest
  releases before it. An installation further behind downloads the whole package.
- **AppImage**: one compressed file, so it is patched whole. Unchanged files' compressed blocks
  match, while changed programs' blocks differ throughout, which leaves about a third of the
  package. Patching the files inside would need building the AppImage's squashfs on the user's
  machine.
- **Nothing is assumed about what changes.** A release that updates Stockfish carries the new
  Stockfish in its patch, about 80 MB, still less than the whole package. A patch that would save
  less than a quarter is not published.

## Update time

Before is the whole package's download; after is the patch's download plus the rebuild
(2.7 s for .deb, macOS and Windows, 4.5 s for the AppImage).

| Platform       | 10 Mbit/s before → after | 50 Mbit/s before → after | 100 Mbit/s before → after |
| -------------- | -----------------------: | -----------------------: | ------------------------: |
| Linux .deb     | 108 s → **8 s**          | 22 s → **4 s**           | 11 s → **3 s**            |
| Linux AppImage | 104 s → **38 s**         | 21 s → **11 s**          | 10 s → **8 s**            |
| macOS          | 95 s → **6 s**           | 19 s → **3 s**           | 10 s → **3 s**            |
| Windows        | 90 s → **8 s**           | 18 s → **4 s**           | 9 s → **3 s**             |

On a fast connection the rebuild, mostly reading the unchanged Stockfish to check it, is most of
what is left.

## When a patch cannot be used

The updater downloads the whole package, as before, when:

- the release has no patch from the installed version;
- the patch is not smaller than the package;
- an installed file the patch starts from is missing or was changed (a deleted Stockfish, an
  antivirus quarantine);
- anything the patch makes does not match its checksum.

The patch itself is checked against the release's signed `SHA256SUMS.txt` like any download, and
what it makes is staged and signature-checked as the whole package's download is. These cases are
covered by `an_update_comes_by_its_patch_and_else_whole`,
`a_patch_is_used_only_when_it_is_the_smaller_download`,
`a_patch_is_staged_as_its_package_installs`, and kchess-delta's tests.

## Reliability added with it

- The update check and the release's checksum files are tried three times, waiting 2 s and then
  4 s, after a dropped connection, a server error or `429`; any other answer fails at once
  (`a_check_tries_again_after_a_server_error`, `the_checksums_are_asked_for_again`). Before, one
  failed check showed an error until the next automatic check six hours later.
- Downloads, patches included, resume where they stopped and are kept across restarts, as
  before.
