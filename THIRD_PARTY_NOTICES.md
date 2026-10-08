# Third-party notices

KChess is licensed under the GPL-3.0-or-later (see [`LICENSE`](LICENSE)). It is built
on the following projects, each under its own license. Their license texts ship with the
packages in `node_modules/` and, for packaged apps, inside the app bundle.

## Code that ships in the app

| Project                                                                     | License          | Used for                                                                        |
| --------------------------------------------------------------------------- | ---------------- | ------------------------------------------------------------------------------- |
| [`@lichess-org/chessground`](https://github.com/lichess-org/chessground)    | GPL-3.0-or-later | The interactive chess board                                                     |
| [`chessops`](https://github.com/niklasf/chessops)                           | GPL-3.0-or-later | Chess rules, FEN, PGN, SAN                                                      |
| [`stockfish`](https://www.npmjs.com/package/stockfish) (WASM build, "lite") | GPL-3.0          | The bundled computer opponent, run as a separate UCI process                    |
| [`electron-updater`](https://github.com/electron-userland/electron-builder) | MIT              | Checks, verifies and installs desktop app updates                               |
| [Electron](https://www.electronjs.org)                                      | MIT              | Desktop shell (includes Chromium and other components under their own licenses) |
| [Nuxt](https://nuxt.com), [Vue](https://vuejs.org), Vue Router              | MIT              | Application framework                                                           |
| [Nuxt UI](https://ui.nuxt.com), [Tailwind CSS](https://tailwindcss.com)     | MIT              | Interface components and styling                                                |
| [Pinia](https://pinia.vuejs.org), [VueUse](https://vueuse.org)              | MIT              | State and utilities                                                             |
| [`@unovis/ts`](https://unovis.dev), `@unovis/vue`                           | Apache-2.0       | The rating chart                                                                |
| [`openapi-fetch`](https://openapi-ts.dev/openapi-fetch/)                    | MIT              | Typed Lichess API client                                                        |
| [Valibot](https://valibot.dev)                                              | MIT              | Validating values that cross the Electron IPC boundary                          |
| [`lru-cache`](https://github.com/isaacs/node-lru-cache)                     | BlueOak-1.0.0    | In-memory cache for profile requests                                            |
| [Lucide icons](https://lucide.dev) (`@iconify-json/lucide`)                 | ISC              | Interface icons                                                                 |

## Game review method

Game review (`src/shared/review.ts`) follows the method Lichess publishes in
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

| Asset                                           | Source and license                                                                                                                                           |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Board themes (`app/assets/boards/`)             | [lichess-org/lila](https://github.com/lichess-org/lila) — AGPL-3.0-or-later; authors: the lila contributors and pirouetti                                    |
| Move and notification sounds                    | [lichess-org/lila](https://github.com/lichess-org/lila) `public/sound/standard` — AGPL-3.0-or-later                                                          |
| Chess pieces (`app/assets/pieces/`)             | [lichess-org/lila](https://github.com/lichess-org/lila) `public/piece` — 11 sets, each under a GPL-compatible license, listed in `app/assets/ATTRIBUTION.md` |
| App color themes (`app/utils/themePresets.ts`)  | [openchamber/openchamber](https://github.com/openchamber/openchamber) built-in themes — MIT; colors only, credits in `app/assets/ATTRIBUTION.md`             |
| App icon (`build/`, `public/`)                  | Original artwork by the KChess authors, generated by `scripts/make-icons.py` — GPL-3.0-or-later like the code                                                |
| Opening names (`src/shared/data/openings.json`) | [lichess-org/chess-openings](https://github.com/lichess-org/chess-openings) — CC0-1.0 (public domain); regenerated by `scripts/make-openings.mjs`            |

| Puzzle database (downloaded on request, sampled into `kchess.db`) | [database.lichess.org](https://database.lichess.org/#puzzles) — CC0 (public domain); the puzzles come from Lichess games and the Lichess puzzle community |

Lichess tournament icons use the unmodified `app/assets/fonts/lichess.woff2` font
from [lila](https://github.com/lichess-org/lila), revision
`de023bec6f0eb65388fc91b45640c09ff6bfec96`. Its contributors and licenses (OFL, MIT,
CC BY 3.0, AGPLv3+) are recorded in `public/licenses/lichess-icons/COPYING.md`,
which ships alongside the upstream AGPL license in the app.

More detail is in [`app/assets/ATTRIBUTION.md`](app/assets/ATTRIBUTION.md).

## Stockfish

The bundled engine is unmodified Stockfish (via the `stockfish` npm package). Its source
is available from <https://github.com/official-stockfish/Stockfish> and the npm package's
repository. The optional native download fetches an official build from the Stockfish
release page and verifies it against the SHA-256 digest GitHub publishes.

## Offline voice recognition

[`vosk-browser`](https://github.com/ccoreilly/vosk-browser), by Ciaran O'Reilly and contributors,
is distributed under Apache-2.0. Its unmodified WebAssembly worker includes
[Vosk](https://github.com/alphacep/vosk-api) and [Kaldi](https://github.com/kaldi-asr/kaldi)
(Apache-2.0), plus its bundled JavaScript dependencies under their respective licenses.
KChess's voice parser, microphone worklet, and UI are original code; no Lichess voice
application code is copied.

The optionally downloaded `vosk-model-small-en-us-0.15` model is from
[Alpha Cephei's model catalog](https://alphacephei.com/vosk/models) and is Apache-2.0.
Its official ZIP archive has SHA-256
`30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498`.
The app verifies this digest on download and repackages the unchanged model files into a
gzip tar archive for the browser loader. The cached archive is verified before reuse. Attribution and the Apache license ship in `public/voice/`.

## Not affiliated

KChess is not affiliated with or endorsed by Lichess or the Stockfish team.

## Development and verification tooling

The following development-only tools do not ship in KChess's runtime application:

- `@nuxt/eslint`, `eslint`, `eslint-config-prettier` — MIT; linting for Nuxt, Vue and TypeScript.
- `vitest`, `@vitejs/plugin-vue`, `@vue/test-utils`, `happy-dom` — MIT; unit and mounted component tests.
- `@playwright/test` — Apache-2.0; Electron UI and packaged-app verification.

Their licenses are included in their npm package distributions.
