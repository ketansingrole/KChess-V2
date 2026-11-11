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

While editing, `cargo xtask check-fast` checks what changed and what depends on it; run it before
opening a pull request. CI runs the full check on every pull request (what runs where is in
[GUARDRAILS.md](GUARDRAILS.md)), and `main` accepts only pull requests whose CI passed. To run it locally as well:

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
  user keeps belong in the core's library. Every core method names a check for each argument in
  its `core_api!` declaration (`crates/kchess-contracts/src/core_api.rs`).
- A new setting is a field of the `Settings` type with its default in
  `crates/kchess-contracts/src/defaults.rs`, its check in `crates/kchess-contracts/src/check/accounts.rs`,
  and its column: a new entry at the end of `MIGRATIONS` in `crates/kchess-core/src/store/migrations.rs`
  (never edit an earlier migration). The store reads and writes every field by its name.
- Be gentle with Lichess. Bulk downloads run one request at a time, and every request names its
  account and kind for the data-usage counters (`Policy::usage` in
  `crates/kchess-core/src/lichess/policy.rs`).

## Licensing of contributions

KChess is licensed under the [GPL-3.0-or-later](LICENSE). By submitting a contribution you
agree that it is licensed under the same terms, and that you have the right to submit it.

**Only contribute work you own or that is clearly open-licensed.** In particular, do not add
images, icons, sounds, or fonts taken from stock sites, design tools (such as Canva), or
other sources whose terms forbid redistribution or use in open-source projects. Add any new
third-party code or asset to [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) with its license.

## Reporting security problems

Please do not open a public issue for a vulnerability; see [SECURITY.md](SECURITY.md).
