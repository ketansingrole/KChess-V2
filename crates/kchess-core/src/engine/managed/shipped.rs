//! The Stockfish release KChess ships, pinned by tag and by each platform's SHA-256. Plain data
//! with no imports: `cargo xtask pins --check` compiles this file too, to tell when a newer
//! release is out.

/// A release fixed in this source, for what KChess ships: the tag and each platform's asset with
/// its SHA-256, so a release changed upstream after it was checked is refused instead of being
/// signed into a KChess package (`fetch_stockfish`, `cargo xtask package`).
pub struct PinnedRelease {
    pub tag: &'static str,
    /// `(platform, arch, asset name, SHA-256)`, as `Target` names them.
    pub assets: &'static [(&'static str, &'static str, &'static str, &'static str)],
}

/// The Stockfish KChess packages, checked 2026-11-06 against the files GitHub served. Move it to
/// a newer release by updating the tag and every digest together.
pub const SHIPPED: PinnedRelease = PinnedRelease {
    tag: "sf_19",
    assets: &[
        (
            "darwin",
            "arm64",
            "stockfish-macos-universal.tar.gz",
            "a1f0e3bcc5a6927a11fe6fc8e54a779754645f3c2bae2cf13420fd1957adaa77",
        ),
        (
            "darwin",
            "x64",
            "stockfish-macos-universal.tar.gz",
            "a1f0e3bcc5a6927a11fe6fc8e54a779754645f3c2bae2cf13420fd1957adaa77",
        ),
        (
            "win32",
            "x64",
            "stockfish-windows-x86-64-universal.zip",
            "3c8bf1f9ea66a09350a40df4f632288285ac206d99f33ab5842c408fc30b48a7",
        ),
        (
            "win32",
            "arm64",
            "stockfish-windows-arm64-universal.zip",
            "8372ad3f0d7276deb2c70f801f541ec7db463219fc6d9c7592864e542aa4f401",
        ),
        (
            "linux",
            "x64",
            "stockfish-linux-x86-64-universal.tar.gz",
            "9defc0d4e55d49c65a6d042f3e571a39fcea499ade6dbe741b53b8c65e03611f",
        ),
        (
            "linux",
            "arm64",
            "stockfish-linux-arm64-universal.tar.gz",
            "fe26cfd1d9db4c8af3d21e24d9ff34cacb31c1f940085a7583da11796f2bac01",
        ),
    ],
};
