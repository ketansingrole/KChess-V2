# KChess Lighthouse and test results — October 5, 2026

Audited all 16 routes in an isolated branded Electron instance using the production renderer and real desktop IPC. The final navigation pass uses deterministic main-process responses for public puzzle, tournament, and broadcast requests; it measures local rendering without live-service rate limits. The original live-service reports are retained in `after-dark/` and `optimized-dark/`. Lighthouse 13.5.0 used its desktop configuration, the native 1320 × 880 window, and provided (unthrottled) loading performance because this is a desktop app with local assets. Dark mode used navigation audits; light mode used snapshots for accessibility, best practices, and SEO. Performance scores describe this local desktop run, not mobile or internet-hosted loading.

All audited pages score **100 for accessibility, best practices, and SEO in both modes**. No failed accessibility audits remain, including the experimental visible-label/name audit. Performance is **100 on all 16 routes in the deterministic navigation pass**.

| Page | Dark performance | Dark accessibility | Light accessibility | Best practices, dark / light | SEO, dark / light |
| --- | ---: | ---: | ---: | ---: | ---: |
| [home](final-dark/home.html) | 100 | 100 | [100](final-light/home.html) | 100 / 100 | 100 / 100 |
| [computer](final-dark/computer.html) | 100 | 100 | [100](final-light/computer.html) | 100 / 100 | 100 / 100 |
| [local](final-dark/local.html) | 100 | 100 | [100](final-light/local.html) | 100 / 100 | 100 / 100 |
| [puzzles](final-dark/puzzles.html) | 100 | 100 | [100](final-light/puzzles.html) | 100 / 100 | 100 / 100 |
| [practice](final-dark/practice.html) | 100 | 100 | [100](final-light/practice.html) | 100 / 100 | 100 / 100 |
| [analysis](final-dark/analysis.html) | 100 | 100 | [100](final-light/analysis.html) | 100 / 100 | 100 / 100 |
| [history](final-dark/history.html) | 100 | 100 | [100](final-light/history.html) | 100 / 100 | 100 / 100 |
| [studies](final-dark/studies.html) | 100 | 100 | [100](final-light/studies.html) | 100 / 100 | 100 / 100 |
| [editor](final-dark/editor.html) | 100 | 100 | [100](final-light/editor.html) | 100 / 100 | 100 / 100 |
| [online](final-dark/online.html) | 100 | 100 | [100](final-light/online.html) | 100 / 100 | 100 / 100 |
| [tournaments](final-dark/tournaments.html) | 100 | 100 | [100](final-light/tournaments.html) | 100 / 100 | 100 / 100 |
| [watch](final-dark/watch.html) | 100 | 100 | [100](final-light/watch.html) | 100 / 100 | 100 / 100 |
| [insights](final-dark/insights.html) | 100 | 100 | [100](final-light/insights.html) | 100 / 100 | 100 / 100 |
| [friends](final-dark/friends.html) | 100 | 100 | [100](final-light/friends.html) | 100 / 100 | 100 / 100 |
| [players](final-dark/players.html) | 100 | 100 | [100](final-light/players.html) | 100 / 100 | 100 / 100 |
| [settings](final-dark/settings.html) | 100 | 100 | [100](final-light/settings.html) | 100 / 100 | 100 / 100 |

## Fixes

- Corrected the moves and chess-board ARIA roles while preserving native button semantics. Updated the existing move-interaction regression tests for the corrected group role.
- Matched the search button's accessible name to its visible text and keyboard hints, and exposed its keyboard shortcuts.
- Improved game-status, analysis-tab, and hovered theme-option contrast in dark mode.
- Improved the default light-mode primary, muted-text, and live-status colors. Updated the default theme preview and authorization-page primary color to match.
- Increased analysis variation buttons to at least 24 × 24 pixels.
- Added page-description metadata and explicit dimensions for the startup logo.
- Lazy-loaded rating charts in Home and puzzle statistics. Home's initial script payload fell from 1,243,617 to 1,035,821 bytes, saving 207,796 bytes (16.7%).

- Enabled Nuxt UI component detection, retaining all 37 detected controls and their dependencies. The shared stylesheet fell from 256,327 to 182,130 bytes (29.0%).
- Emit chess-piece SVGs as separate assets instead of embedding all sets in JavaScript. Boards fetch the pieces they actually display; settings previews and GIF export retain access to every set.
- Lazy-load inactive puzzle/practice tabs, optional settings panels, and the command palette.
- Generate a 144-pixel splash asset for its 72-pixel display, reducing the splash raster from 34,453 to 7,859 bytes.
- Fixed light-mode status/error badge contrast and primary soft-control contrast. Default previews and the authorization page use the same primary.
- Added the account name to its menu button's accessible name, labelled the search-results listbox, and exposed toast focus proxies to assistive technology while retaining keyboard traversal.

| Initial JavaScript payload | Previous pass | Final pass | Reduction |
| --- | ---: | ---: | ---: |
| Home | 1,035,821 bytes | 952,177 bytes | 83,644 bytes (8.1%) |
| Puzzles | 1,361,741 bytes | 1,043,813 bytes | 317,928 bytes (23.3%) |
| Practice | 1,406,394 bytes | 1,014,348 bytes | 392,046 bytes (27.9%) |

The initial dark-mode accessibility scores included 88 for Analysis, 91 for Computer, 95 for Local play, and 96 for Practice, Editor, and Settings. The light-mode diagnostic run exposed the shared palette contrast failures now fixed.

## Verification

- npm run check: formatting, lint, types, smoke tests, and all 359 unit tests passed across 38 test files.
- npm run build: passed.
- npm run test:e2e: all 28 production Electron tests passed, including search naming and keyboard notification dismissal.
- git diff --check: passed.
- Reviewed Home and Analysis screenshots to verify the palette and enlarged move controls.

## Expanded verification and remaining platform diagnostics

The final checks include 40 additional light/dark snapshots covering all 13 settings sections, the five puzzle tabs, the rated account-required state, and the search dialog. Another 12 snapshots cover a custom light/dark palette, connected-account records with expired authorization, and import/search dialogs. All these snapshots score 100 for accessibility, best practices, and SEO, including the experimental visible-label/name audit. Reports are in `expanded-states/` and `custom-connected/`.

Puzzles now score 100 with approximately 0.2-second LCP in the deterministic pass. Live Lichess response time is separate: repeated live navigation audits encountered HTTP 429, and Lighthouse measured the late error message as LCP. The app must still wait for a real online puzzle; this report does not claim that local code can remove service latency. Gameplay, account transitions, persistence, offline puzzles, voice, updates, and exports also pass the production desktop regression suite.

The remaining unused-code diagnostics include shared framework/runtime functions and styles needed by other states. Component detection and tab/asset splitting remove avoidable loading without deleting functional code. The unsuppressed reports retain those diagnostics. HTTP cache/compression suggestions concern the temporary local audit server; the packaged app reads local assets through its custom protocol. Electron's back/forward-cache delegate restrictions are outside the renderer's control. Arbitrary user-created palettes and every possible account/game state are not exhaustively certified.

Existing watch-page, watch-store, watch-test, and move-alignment edits were preserved. No dependency/version changes, commits, tags, or releases were made. Reports are saved under the repository's ignored docs directory.
