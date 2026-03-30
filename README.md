# KChess

KChess is a Rust desktop chess app built with GPUI.

## Highlights

- Play against the built-in computer opponent with interactive board annotations.
- Move history timeline with back/forward stepping for position review.
- History page for synced Lichess games, including account/rated/result filters and paging.
- Settings page for board theme presets/custom colors and Lichess account management.
- Sidebar + search-driven navigation across Play, History, and Settings panes.

## Workspace Layout

- `src/` - app shell, page views, Lichess sync/auth/state, storage, and UI wiring
- `crates/kchess_board/` - board state engine and GPUI board view
- `crates/kchess_board_api/` - API wrapper crate on top of `kchess_board`
- `assets/` - app icons and other UI assets
- `scripts/build_macos_app.sh` - builds a macOS `.app` bundle

## Lichess Integration

- Supports adding tracked public Lichess usernames and syncing recent games.
- Supports browser-based OAuth connect flow for your own Lichess account.
- Stores synced game history locally and keeps account sync cursors per user.

## Local Data and Credentials

- SQLite storage path: `~/.kchess/kchess.db`
- OAuth access tokens are stored via OS credential storage (keychain), not in the SQLite DB.

## Prerequisites

- Rust stable toolchain
- macOS (required for the `.app` bundle script)

## Run in Development

```bash
cargo run --bin KChess
```

## Quality Checks

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test --workspace
```

## Build macOS App Bundle

```bash
./scripts/build_macos_app.sh
open "target/macos-bundle/KChess.app"
```

## Keyboard and Board Controls

- `Cmd+F`: open search
- `Cmd+,`: open Settings
- `Left` / `Right`: step backward/forward through move history (Play view)
- `Up` / `Down`: navigate search suggestions
- `Enter`: confirm selected search result
- `Esc`: close search
- Right-click drag on board: draw/remove arrows
- Right-click square: toggle square highlight
- Left-click square: clear all board annotations (highlights + arrows)

## Icon Sources

- Primary source bundle used by app/UI: `assets/app/kchess.icon`
