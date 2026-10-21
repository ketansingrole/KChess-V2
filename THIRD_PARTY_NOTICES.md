# Third-party notices

KChess is licensed under the GPL-3.0-or-later (see [`LICENSE`](LICENSE)). It is built
on the following projects, each under its own license. Their license texts ship with the
packages in `node_modules/` and, for packaged apps, inside the app bundle.

## Code that ships in the app

| Project                                                                                                                                           | License                                                                | Used for                                                                          |
| ------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| [`@lichess-org/chessground`](https://github.com/lichess-org/chessground)                                                                          | GPL-3.0-or-later                                                       | The interactive chess board                                                       |
| [`chessops`](https://github.com/niklasf/chessops)                                                                                                 | GPL-3.0-or-later                                                       | Chess rules, FEN, PGN and SAN semantics, ported to Rust in `crates/kchess-domain` |
| [`stockfish`](https://www.npmjs.com/package/stockfish) (WASM build, "lite")                                                                       | GPL-3.0                                                                | The bundled computer opponent, run as a separate UCI process                      |
| [`electron-updater`](https://github.com/electron-userland/electron-builder)                                                                       | MIT                                                                    | Checks, verifies and installs desktop app updates                                 |
| [Electron](https://www.electronjs.org)                                                                                                            | MIT                                                                    | Desktop shell (includes Chromium and other components under their own licenses)   |
| [Nuxt](https://nuxt.com), [Vue](https://vuejs.org), Vue Router (`vue-router`)                                                                     | MIT                                                                    | Application framework                                                             |
| [Nuxt UI](https://ui.nuxt.com) (`@nuxt/ui`), [Tailwind CSS](https://tailwindcss.com) (`tailwindcss`)                                              | MIT                                                                    | Interface components and styling                                                  |
| [Pinia](https://pinia.vuejs.org), [VueUse](https://vueuse.org) (`@vueuse/core`)                                                                   | MIT                                                                    | State and utilities                                                               |
| [`openapi-fetch`](https://openapi-ts.dev/openapi-fetch/)                                                                                          | MIT                                                                    | Typed Lichess API client                                                          |
| [`lru-cache`](https://github.com/isaacs/node-lru-cache)                                                                                           | BlueOak-1.0.0                                                          | In-memory cache for profile requests                                              |
| [Lucide icons](https://lucide.dev) (`@iconify-json/lucide`)                                                                                       | ISC                                                                    | Interface icons                                                                   |
| [`shakmaty`](https://github.com/niklasf/shakmaty)                                                                                                 | GPL-3.0-or-later                                                       | Chess and variant rules in the core's native rules (`crates/kchess-domain`)       |
| [`serde`](https://serde.rs), `serde_json`                                                                                                         | MIT OR Apache-2.0                                                      | Documents passed between the core and its native rules                            |
| [`thiserror`](https://github.com/dtolnay/thiserror)                                                                                               | MIT OR Apache-2.0                                                      | Typed rule errors (`DomainError`) in the native rules (`crates/kchess-domain`)    |
| [`memchr`](https://github.com/BurntSushi/memchr)                                                                                                  | MIT OR Unlicense                                                       | Scanning the puzzle database CSV in the native rules                              |
| [napi-rs](https://napi.rs) (`napi`, `napi-derive`)                                                                                                | MIT                                                                    | Loading the native rules into Node and Electron                                   |
| [`rusqlite`](https://github.com/rusqlite/rusqlite) (bundled SQLite)                                                                               | MIT (SQLite: public domain)                                            | The core's game library, studies, settings and puzzle database                    |
| [`tokio`](https://tokio.rs), `tokio-util`                                                                                                         | MIT                                                                    | The core's async runtime, engine processes and cancellation                       |
| [`reqwest`](https://github.com/seanmonstar/reqwest)                                                                                               | MIT OR Apache-2.0                                                      | HTTPS requests and streams to Lichess and engine downloads                        |
| [`futures-util`](https://github.com/rust-lang/futures-rs)                                                                                         | MIT OR Apache-2.0                                                      | Reading streamed responses in the core                                            |
| [`ruzstd`](https://github.com/KillingSpark/zstd-rs)                                                                                               | MIT                                                                    | Decompressing the downloaded puzzle database                                      |
| [`getrandom`](https://github.com/rust-random/getrandom), [`uuid`](https://github.com/uuid-rs/uuid)                                                | MIT OR Apache-2.0                                                      | Random ids for studies and chapters                                               |
| [`sha2`](https://github.com/RustCrypto/hashes), [`base64`](https://github.com/marshallpierce/rust-base64)                                         | MIT OR Apache-2.0                                                      | Engine download checks and the Lichess sign-in challenge                          |
| [`flate2`](https://github.com/rust-lang/flate2-rs), [`tar`](https://github.com/composefs/tar-rs) (crate), [`zip`](https://github.com/zip-rs/zip2) | MIT OR Apache-2.0 (`zip`: MIT)                                         | Unpacking a downloaded engine                                                     |
| [`tempfile`](https://github.com/Stebalien/tempfile)                                                                                               | MIT OR Apache-2.0                                                      | A private staging directory for engine downloads                                  |
| [`rustix`](https://github.com/bytecodealliance/rustix), [`libc`](https://github.com/rust-lang/libc)                                               | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT; MIT OR Apache-2.0 | Engine process signals and file checks on macOS and Linux                         |

The native rules (`@kchess/native`) are compiled Rust. The license text of every crate compiled
into them, including indirect dependencies, ships beside the module as `THIRD_PARTY_LICENSES.txt`
(generated by `tooling/rust-licenses.mjs` on each native build).

## Game review method

Game review (`crates/kchess-wasm/js/review.ts`) follows the method Lichess publishes in
[lichess-org/lila](https://github.com/lichess-org/lila) (AGPL-3.0-or-later): its winning-chance
curve, the thresholds for inaccuracies, mistakes and blunders (including its forced-mate rules),
and its accuracy formula. The formulas are reimplemented here; no lila code is copied. Analysis
fetched from Lichess for a game comes from the Lichess API.

## Voice model preparation

`fflate` (MIT) and `tar` (ISC) verify and repackage the downloaded speech model in the
desktop process on first use; they do not run in the renderer's recognition pipeline.

`@lichess-org/types` (AGPL-3.0-or-later) provides TypeScript type definitions for the
Lichess API. It contains no runtime code and nothing from it is included in the built app.
The remaining development dependencies (TypeScript, Vite, electron-builder, Prettier, and
similar) are MIT or Apache-2.0 and are not distributed with the app.

## Assets

| Asset                                                       | Source and license                                                                                                                                                                                                                           |
| ----------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Board themes (`apps/desktop/app/assets/boards/`)            | [lichess-org/lila](https://github.com/lichess-org/lila) — AGPL-3.0-or-later; authors: the lila contributors and pirouetti                                                                                                                    |
| Move and notification sounds                                | [lichess-org/lila](https://github.com/lichess-org/lila) `apps/desktop/public/sound/standard` — AGPL-3.0-or-later                                                                                                                             |
| Chess pieces (`apps/desktop/app/assets/pieces/`)            | [lichess-org/lila](https://github.com/lichess-org/lila) `apps/desktop/public/piece` — 11 sets, each under a GPL-compatible license, listed in `apps/desktop/app/assets/ATTRIBUTION.md`                                                       |
| App color themes (`apps/desktop/app/utils/themePresets.ts`) | [openchamber/openchamber](https://github.com/openchamber/openchamber) built-in themes — MIT; colors only, credits in `apps/desktop/app/assets/ATTRIBUTION.md`                                                                                |
| App icon (`apps/desktop/build/`, `apps/desktop/public/`)    | Original artwork by the KChess authors, generated by `tooling/make-icons.py` — GPL-3.0-or-later like the code                                                                                                                                |
| Opening names (`crates/kchess-domain/data/openings.json`)   | [lichess-org/chess-openings](https://github.com/lichess-org/chess-openings) — CC0-1.0 (public domain); regenerated by `tooling/make-openings.mjs`                                                                                            |
| Endgame lessons (`crates/kchess-domain/data/lessons.json`)  | Original text and positions by the KChess authors — GPL-3.0-or-later like the code; the chapter list follows the topics of lichess.org/practice, whose lesson text and positions are not used. Saavedra's 1895 study is in the public domain |

| Puzzle database (downloaded on request, sampled into `puzzles.db`) | [database.lichess.org](https://database.lichess.org/#puzzles) — CC0 (public domain); the puzzles, their themes and opening tags come from Lichess games and the Lichess puzzle community |

Lichess tournament icons use the unmodified `apps/desktop/app/assets/fonts/lichess.woff2` font
from [lila](https://github.com/lichess-org/lila), revision
`de023bec6f0eb65388fc91b45640c09ff6bfec96`. Its contributors and licenses (OFL, MIT,
CC BY 3.0, AGPLv3+) are recorded in `apps/desktop/public/licenses/lichess-icons/COPYING.md`,
which ships alongside the upstream AGPL license in the app.

More detail is in [`apps/desktop/app/assets/ATTRIBUTION.md`](apps/desktop/app/assets/ATTRIBUTION.md).

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

The bundled engine is unmodified Stockfish (via the `stockfish` npm package). Its source
is available from <https://github.com/official-stockfish/Stockfish> and the npm package's
repository. The optional native download fetches an official build from the Stockfish
release page and verifies it against the SHA-256 digest GitHub publishes.

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

The following development-only tools do not ship in KChess's runtime application:

- `@nuxt/eslint`, `eslint`, `eslint-config-prettier` — MIT; linting for Nuxt, Vue and TypeScript.
- `vitest`, `@vitejs/plugin-vue`, `@vue/test-utils`, `happy-dom` — MIT; unit and mounted component tests.
- `@playwright/test` — Apache-2.0; Electron UI and packaged-app verification.

Their licenses are included in their npm package distributions.
