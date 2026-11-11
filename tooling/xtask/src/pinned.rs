//! Every file the build downloads by a fixed version, with its SHA-256: the builders fetch these
//! (`util::pinned` refuses any other bytes), and `cargo xtask pins --check` asks upstream whether
//! a newer one is out. CI keys its engine cache on this file and on the shipped Stockfish's own pin
//! (`crates/kchess-core/src/engine/managed/shipped.rs`, which the core installs at run time).
//!
//! A pin changes only in a reviewed pull request, which CI packages and tests.

// The core's pin, compiled here as well: plain data, so this reads the very constants the core
// installs from.
#[path = "../../../crates/kchess-core/src/engine/managed/shipped.rs"]
mod shipped;

/// Where a pinned file comes from upstream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A release asset: `https://github.com/<repo>/releases/download/<version>/<file>`.
    Release,
    /// GitHub's archive of a commit (`version`) on `branch`.
    Commit { branch: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pin {
    pub name: &'static str,
    /// The GitHub repository, `owner/name`.
    pub repo: &'static str,
    /// The release tag, or the commit.
    pub version: &'static str,
    /// The release asset; for a commit, the archive's name.
    pub file: &'static str,
    pub sha256: &'static str,
    pub source: Source,
}

impl Pin {
    pub fn url(&self) -> String {
        let Pin {
            repo,
            version,
            file,
            ..
        } = self;
        match self.source {
            Source::Release => {
                format!("https://github.com/{repo}/releases/download/{version}/{file}")
            }
            Source::Commit { .. } => format!("https://codeload.github.com/{repo}/tar.gz/{version}"),
        }
    }
}

pub const APPIMAGETOOL: Pin = Pin {
    name: "appimagetool",
    repo: "AppImage/appimagetool",
    version: "1.9.1",
    file: "appimagetool-x86_64.AppImage",
    sha256: "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0",
    source: Source::Release,
};

/// The runtime appimagetool puts at the front of every AppImage: the launcher that mounts the
/// app and starts it. Without `--runtime-file`, appimagetool would fetch the moving `continuous`
/// release on every build, unverified.
pub const APPIMAGE_RUNTIME: Pin = Pin {
    name: "the AppImage runtime",
    repo: "AppImage/type2-runtime",
    version: "20251108",
    file: "runtime-x86_64",
    sha256: "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d",
    source: Source::Release,
};

/// Fairy-Stockfish, the variant engine, built from this commit (`fairy.rs`).
pub const FAIRY_STOCKFISH: Pin = Pin {
    name: "Fairy-Stockfish",
    repo: "fairy-stockfish/Fairy-Stockfish",
    version: "2f1959e1f0b44d4648ff35015b89d8e918a5c746",
    file: "Fairy-Stockfish.tar.gz",
    sha256: "f226f3c67cb30e87b6990ce7d38cf29cbaca59709f2dba23f421fafdecbb67e5",
    source: Source::Commit { branch: "master" },
};

/// The official prebuilt libvosk (`voice.rs`). The newest are 0.3.45 for Linux and Windows
/// (Dec 2022) and 0.3.42 for macOS (May 2022).
pub const VOSK_MACOS: Pin = Pin {
    name: "libvosk for macOS",
    repo: "alphacep/vosk-api",
    version: "v0.3.42",
    file: "vosk-osx-0.3.42.zip",
    sha256: "65395f196c9d0583d79949142b25560acaf9c295f36284e18433097f3adb0ea1",
    source: Source::Release,
};
pub const VOSK_LINUX_X64: Pin = Pin {
    name: "libvosk for Linux",
    repo: "alphacep/vosk-api",
    version: "v0.3.45",
    file: "vosk-linux-x86_64-0.3.45.zip",
    sha256: "bbdc8ed85c43979f6443142889770ea95cbfbc56cffb5c5dcd73afa875c5fbb2",
    source: Source::Release,
};
pub const VOSK_WINDOWS_X64: Pin = Pin {
    name: "libvosk for Windows",
    repo: "alphacep/vosk-api",
    version: "v0.3.45",
    file: "vosk-win64-0.3.45.zip",
    sha256: "f1dcc9cca460630f81ea8f71794f69c80bed6556d2a4e6237b5785e1d2dff34b",
    source: Source::Release,
};

pub fn stockfish_tag() -> &'static str {
    shipped::SHIPPED.tag
}

/// The Stockfish KChess ships: one pin per distinct asset of the core's release.
pub fn stockfish() -> Vec<Pin> {
    let mut pins: Vec<Pin> = Vec::new();
    for (_, _, file, sha256) in shipped::SHIPPED.assets {
        if pins.iter().all(|pin| pin.file != *file) {
            pins.push(Pin {
                name: "Stockfish",
                repo: "official-stockfish/Stockfish",
                version: shipped::SHIPPED.tag,
                file,
                sha256,
                source: Source::Release,
            });
        }
    }
    pins
}

/// Every pin, and the file it is kept in.
pub fn all() -> Vec<(Pin, &'static str)> {
    let here = "tooling/xtask/src/pinned.rs";
    let mut pins: Vec<(Pin, &'static str)> = [
        APPIMAGETOOL,
        APPIMAGE_RUNTIME,
        FAIRY_STOCKFISH,
        VOSK_MACOS,
        VOSK_LINUX_X64,
        VOSK_WINDOWS_X64,
    ]
    .into_iter()
    .map(|pin| (pin, here))
    .collect();
    pins.extend(
        stockfish()
            .into_iter()
            .map(|pin| (pin, "crates/kchess-core/src/engine/managed/shipped.rs")),
    );
    pins
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pin_is_a_digest_and_its_file_exists() {
        for (pin, file) in all() {
            assert_eq!(pin.sha256.len(), 64, "{}", pin.name);
            assert!(
                pin.sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                "{}",
                pin.name
            );
            assert!(crate::util::root().join(file).is_file(), "{file}");
        }
        assert_eq!(
            APPIMAGE_RUNTIME.url(),
            "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64"
        );
        assert_eq!(
            FAIRY_STOCKFISH.url(),
            format!(
                "https://codeload.github.com/fairy-stockfish/Fairy-Stockfish/tar.gz/{}",
                FAIRY_STOCKFISH.version
            )
        );
        // The macOS and x64 assets of one universal build are one download.
        let stockfish = stockfish();
        assert!(stockfish.len() >= 4, "{stockfish:?}");
        assert!(
            stockfish
                .iter()
                .all(|pin| pin.version == shipped::SHIPPED.tag)
        );
    }
}
