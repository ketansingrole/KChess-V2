# Third-party notices

KChess is licensed under the GPL-3.0-or-later (see [`LICENSE`](LICENSE)). It is built
on the following projects, each under its own license. Every package carries the license text
of every crate compiled into it, including indirect dependencies, as `THIRD_PARTY_LICENSES.txt`
beside the app (written by `cargo xtask licenses` when it is packaged).

## Code that ships in the app

The app (`apps/gpui`), its core service (`kchess-service`), the command line (`kchess-cli`) and
the speech recognition helper (`kchess-voice`) are Rust. `cargo xtask notices` checks that every
crate they depend on directly is named here.

| Project | License | Used for |
| ------- | ------- | -------- |
| [`shakmaty`](https://github.com/niklasf/shakmaty)                                                                                                 | GPL-3.0-or-later                                                       | Chess and variant rules in the core's native rules (`crates/kchess-domain`)            |
| [`serde`](https://serde.rs), `serde_json`                                                                                                         | MIT OR Apache-2.0                                                      | Documents passed between the core and its native rules                                 |
| [`thiserror`](https://github.com/dtolnay/thiserror)                                                                                               | MIT OR Apache-2.0                                                      | Typed rule errors (`DomainError`) in the native rules (`crates/kchess-domain`)         |
| [`memchr`](https://github.com/BurntSushi/memchr)                                                                                                  | MIT OR Unlicense                                                       | Scanning the puzzle database CSV in the native rules                                   |
| [`rusqlite`](https://github.com/rusqlite/rusqlite) (bundled SQLite)                                                                               | MIT (SQLite: public domain)                                            | The core's game library, studies, settings and puzzle database                         |
| [`tokio`](https://tokio.rs), `tokio-util`                                                                                                         | MIT                                                                    | The core's async runtime, engine processes and cancellation                            |
| [`reqwest`](https://github.com/seanmonstar/reqwest)                                                                                               | MIT OR Apache-2.0                                                      | HTTPS requests and streams to Lichess and engine downloads                             |
| [`futures-util`](https://github.com/rust-lang/futures-rs)                                                                                         | MIT OR Apache-2.0                                                      | Reading streamed responses in the core                                                 |
| [`ruzstd`](https://github.com/KillingSpark/zstd-rs)                                                                                               | MIT                                                                    | Decompressing the downloaded puzzle database                                           |
| [`getrandom`](https://github.com/rust-random/getrandom), [`uuid`](https://github.com/uuid-rs/uuid)                                                | MIT OR Apache-2.0                                                      | Random ids for studies and chapters                                                    |
| [`sha2`](https://github.com/RustCrypto/hashes), [`base64`](https://github.com/marshallpierce/rust-base64)                                         | MIT OR Apache-2.0                                                      | Engine download checks and the Lichess sign-in challenge                               |
| [`flate2`](https://github.com/rust-lang/flate2-rs), [`tar`](https://github.com/composefs/tar-rs) (crate), [`zip`](https://github.com/zip-rs/zip2) | MIT OR Apache-2.0 (`zip`: MIT)                                         | Unpacking a downloaded engine                                                          |
| [`tempfile`](https://github.com/Stebalien/tempfile)                                                                                               | MIT OR Apache-2.0                                                      | A private staging directory for engine downloads                                       |
| [`rustix`](https://github.com/bytecodealliance/rustix), [`libc`](https://github.com/rust-lang/libc)                                               | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT; MIT OR Apache-2.0 | Engine process signals and file checks on macOS and Linux                              |
| [`aes-gcm`](https://github.com/RustCrypto/AEADs), [`keyring`](https://github.com/open-source-cooperative/keyring-rs)                              | Apache-2.0 OR MIT; MIT OR Apache-2.0                                   | Sealing Lichess logins in the profile with a key kept in the system's credential store |
| [Stockfish](https://stockfishchess.org) (official native build)                                                                                                   | GPL-3.0                                | The bundled computer opponent, beside the app, run as a separate UCI process                                                  |
| [GPUI](https://github.com/zed-industries/zed) (`gpui-pre`), [GPUI Kit](https://github.com/longbridge/gpui-kit) (`gpui-kit`, `gpui-component`, `gpui-base`)        | Apache-2.0                             | The window, rendering and interface components; `gpui-pre` ships with a window zoom patch (`vendor/gpui-pre/KCHESS-PATCH.md`) |
| [`rodio`](https://github.com/RustAudio/rodio), [`cpal`](https://github.com/RustAudio/cpal), [Symphonia](https://github.com/pdeljanov/Symphonia)                   | MIT OR Apache-2.0; Apache-2.0; MPL-2.0 | Playing sounds, and the microphone for voice input                                                                            |
| [`resvg`](https://github.com/linebender/resvg) (`usvg`), [`tiny-skia`](https://github.com/RazrFalcon/tiny-skia), [`fontdb`](https://github.com/RazrFalcon/fontdb) | Apache-2.0 OR MIT; BSD-3-Clause; MIT   | Drawing pieces, and the pictures a game exports                                                                               |
| [`image`](https://github.com/image-rs/image) (`png`, `gif`, `zune-jpeg`)                                                                                          | MIT OR Apache-2.0                      | Reading board images and encoding exported PNGs and GIFs                                                                      |
| [`chrono`](https://github.com/chronotope/chrono), [`dirs`](https://github.com/soc/dirs-rs), [`futures`](https://github.com/rust-lang/futures-rs)                  | MIT OR Apache-2.0                      | Local dates, the Downloads folder, channels from the core                                                                     |
| [`rust-embed`](https://pyrossh.dev/repos/rust-embed)                                                                                                              | MIT                                    | The boards, pieces, sounds and icons inside the app                                                                           |
| [`notify-rust`](https://github.com/hoodie/notify-rust) (`mac-notification-sys`, `tauri-winrt-notification`)                                                       | MIT OR Apache-2.0                      | Desktop notifications while KChess is in the background                                                                       |
| [`objc2`](https://github.com/madsmtm/objc2) (`objc2-foundation`, `objc2-av-foundation`, `block2`)                                                                 | MIT                                    | Reading and asking for microphone access on macOS                                                                             |
| [Lucide icons](https://lucide.dev) | ISC | Interface icons (SVG files in `apps/gpui/assets/icons`) |
| [`regex`](https://github.com/rust-lang/regex) | MIT OR Apache-2.0 | Matching typed and spoken input in the app |
| [`objc2-app-kit`](https://github.com/madsmtm/objc2) | MIT | The Dock icon on macOS |
| [`libloading`](https://github.com/nagisa/rust_libloading) | ISC | Loading libvosk into the speech recognition helper |
| [`rusty-leveldb`](https://github.com/dermesser/leveldb-rs) | MIT | Reading what the Electron app kept in its `localStorage`, once |
| [`aes`](https://github.com/RustCrypto/block-ciphers), [`cbc`](https://github.com/RustCrypto/block-modes), [`pbkdf2`](https://github.com/RustCrypto/password-hashes), [`sha1`](https://github.com/RustCrypto/hashes) | MIT OR Apache-2.0 | Opening Lichess logins the Electron app sealed with `safeStorage`, to seal them again |
| [`secret-service`](https://github.com/hwchen/secret-service-rs) | MIT OR Apache-2.0 | Finding the Electron app's `safeStorage` key on Linux |
| [`windows-sys`](https://github.com/microsoft/windows-rs) | MIT OR Apache-2.0 | Opening the Electron app's `safeStorage` key on Windows (DPAPI) |

## Game review method

Game review (`crates/kchess-wasm/js/review.ts`) follows the method Lichess publishes in
[lichess-org/lila](https://github.com/lichess-org/lila) (AGPL-3.0-or-later): its winning-chance
curve, the thresholds for inaccuracies, mistakes and blunders (including its forced-mate rules),
and its accuracy formula. The formulas are reimplemented here; no lila code is copied. Analysis
fetched from Lichess for a game comes from the Lichess API.

## Voice model preparation

The core verifies the downloaded speech model against its pinned SHA-256 and unpacks it with
`flate2` and `tar` (listed above) on first use.

## Assets

| Asset                                                      | Source and license                                                                                                                                                                                                                           |
| ---------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Board themes (`apps/gpui/assets/boards/`)           | [lichess-org/lila](https://github.com/lichess-org/lila) — AGPL-3.0-or-later; authors: the lila contributors and pirouetti                                                                                                                    |
| Move and notification sounds                               | [lichess-org/lila](https://github.com/lichess-org/lila) `public/sound/standard` — AGPL-3.0-or-later                                                                                                                                          |
| Chess pieces (`apps/gpui/assets/pieces/`)           | [lichess-org/lila](https://github.com/lichess-org/lila) `public/piece` — 11 sets, each under a GPL-compatible license, listed in `apps/gpui/assets/ATTRIBUTION.md`                                                                    |
| App color themes (`crates/kchess-domain/data/themes.json`) | [openchamber/openchamber](https://github.com/openchamber/openchamber) built-in themes — MIT; colors only, credits in `apps/gpui/assets/ATTRIBUTION.md`                                                                                |
| App icon (`apps/gpui/build/`, `apps/gpui/assets/`)         | Original artwork by the KChess authors, generated by `cargo xtask app-icon` — GPL-3.0-or-later like the code                                                                                                                                   |
| Opening names (`crates/kchess-domain/data/openings.json`)  | [lichess-org/chess-openings](https://github.com/lichess-org/chess-openings) — CC0-1.0 (public domain); regenerated by `tooling/make-openings.mjs`                                                                                            |
| Endgame lessons (`crates/kchess-domain/data/lessons.json`) | Original text and positions by the KChess authors — GPL-3.0-or-later like the code; the chapter list follows the topics of lichess.org/practice, whose lesson text and positions are not used. Saavedra's 1895 study is in the public domain |

| Puzzle database (downloaded on request, sampled into `puzzles.db`) | [database.lichess.org](https://database.lichess.org/#puzzles) — CC0 (public domain); the puzzles, their themes and opening tags come from Lichess games and the Lichess puzzle community |

More detail is in [`apps/gpui/assets/ATTRIBUTION.md`](apps/gpui/assets/ATTRIBUTION.md).

## Content fetched at runtime

Opening pages show the Wikibooks
[Chess Opening Theory](https://en.wikibooks.org/wiki/Chess_Opening_Theory) page of the line,
if it has one. The text is under
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) by the Wikibooks contributors.
It is not bundled: the core fetches it as plain text when an opening page opens and caches
it with the explorer's saved lookups. It is shown as an excerpt (the page's title line,
editing notes and reference sections are left out), marked as such, with links to the source
page and the licence. Opening popularity comes from the Lichess opening explorer API.

## Stockfish

The bundled engine is an unmodified official Stockfish build, fetched from the Stockfish release
page when KChess is packaged and verified against the SHA-256 digest GitHub publishes, as the
app's own engine download is. Its source is available from
<https://github.com/official-stockfish/Stockfish>.

## Offline voice recognition

The speech recognition helper (`crates/kchess-voice`) loads `libvosk`, the official prebuilt
[Vosk](https://github.com/alphacep/vosk-api) library by Alpha Cephei Inc. and contributors
(Apache-2.0), which includes [Kaldi](https://github.com/kaldi-asr/kaldi) and
[OpenFst](https://www.openfst.org) (Apache-2.0). KChess ships it unchanged from the Vosk
release archives (0.3.42 for macOS, of which only the arm64 slice is kept; 0.3.45 for Linux and
Windows), each pinned by SHA-256 in `tooling/voice-runtime.mjs`. On Windows, libvosk needs the
MinGW-w64 runtime beside it: `libstdc++-6.dll` and `libgcc_s_seh-1.dll` (GPL-3.0 with the GCC
Runtime Library Exception) and `libwinpthread-1.dll` (MIT and BSD-3-Clause), from the same
archive. Attribution and the Apache license ship beside the helper (`voice/NOTICE.txt`,
`voice/APACHE-2.0.txt`). KChess's voice parser, microphone worklet, helper and UI are original
code; no Lichess voice application code is copied.

The optionally downloaded `vosk-model-small-en-us-0.15` model is from
[Alpha Cephei's model catalog](https://alphacephei.com/vosk/models) and is Apache-2.0.
Its official ZIP archive has SHA-256
`30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498`.
The app verifies this digest on download and unpacks the unchanged model files into the
app's data folder, recording each file's SHA-256; the cached files are verified before reuse.

## Not affiliated

KChess is not affiliated with or endorsed by Lichess or the Stockfish team.

## Development and verification tooling

Development-only crates (`cargo xtask`, test-only dependencies such as `regex-syntax`) and the
tools CI installs (`cargo-llvm-cov`, NSIS, appimagetool, cargo-fuzz) do not ship in KChess.
