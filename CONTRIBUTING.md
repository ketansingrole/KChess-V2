# Contributing to KChess

Thanks for helping! Bug reports, fixes, and ideas are all welcome.

## Getting started

KChess is Rust. Install Rust through rustup (the toolchain is pinned in `rust-toolchain.toml`);
on Linux, also the libraries the app builds against:

```bash
sudo apt-get install libasound2-dev libfontconfig-dev libwayland-dev libx11-xcb-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libvulkan-dev
```

Then:

```bash
cargo xtask install-hooks
KCHESS_USER_DATA_DIR=/tmp/kchess-dev cargo run -p kchess-gpui
```

While editing, `cargo xtask check-fast` checks only what changed. Pushing to `main` runs
`cargo xtask check` through the pre-push hook. Before opening a pull request, run the same check
CI runs:

```bash
cargo xtask check
```

To reproduce CI's Linux build locally, start Docker (on macOS, `brew install colima docker` and
`colima start`) and use the image built from the repository pins (`tooling/linux/Dockerfile`).

## Guidelines

- Keep changes focused; one topic per pull request. Describe what changed and why, and
  how you checked it (screenshots help for UI changes).
- Match the surrounding code: naming, comment density, and idiom. `cargo fmt` formats it.
- Put headless features in the core (`crates/kchess-core`) behind `CoreApi` (`kchess-contracts`),
  pure chess or training rules in `crates/kchess-domain`, and views in `apps/gpui`. Documents a
  user keeps belong in the core's library. Every core method validates its arguments
  (`crates/kchess-domain/src/misc/contracts.rs`).
- New settings need three things: the `Settings` type, the `SETTINGS_KEYS` list in
  `crates/kchess-core/src/store/mod.rs`, and a new entry at the end of `MIGRATIONS` in
  `crates/kchess-core/src/store/migrations.rs` (never edit an earlier migration).
- Be gentle with Lichess. Bulk downloads run one request at a time, and requests should be
  attributed for the data-usage counters (`with_usage` in `crates/kchess-core/src/lichess`).

## Licensing of contributions

KChess is licensed under the [GPL-3.0-or-later](LICENSE). By submitting a contribution you
agree that it is licensed under the same terms, and that you have the right to submit it.

**Only contribute work you own or that is clearly open-licensed.** In particular, do not add
images, icons, sounds, or fonts taken from stock sites, design tools (such as Canva), or
other sources whose terms forbid redistribution or use in open-source projects. Add any new
third-party code or asset to [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) with its license.

## Reporting security problems

Please do not open a public issue for a vulnerability; see [SECURITY.md](SECURITY.md).
