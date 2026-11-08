//! A release's packages by name: `KChess-<version>-<platform>-<arch><suffix>`. The packaging
//! writes these names, the release tooling reads them, and installed apps ask a release for the
//! one they were installed from, so they never change.

/// What every package's name starts with.
pub const PREFIX: &str = "KChess";

/// One kind of package a release can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Asset {
    /// The macOS bundle zipped: what the updater installs.
    MacZip,
    /// The macOS disk image: what people download.
    MacDmg,
    /// The Mac App Store's installer package, which only App Store Connect takes.
    MasPkg,
    /// The Windows per-user installer.
    WindowsInstaller,
    /// The Windows installer without its files, which a Windows update patch carries.
    WindowsUpdateInstaller,
    AppImage,
    Deb,
    Snap,
}

/// The processor a package is built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Arch {
    Arm64,
    X64,
}

impl Arch {
    /// This machine's, when packages are made for it.
    pub fn current() -> Option<Arch> {
        match std::env::consts::ARCH {
            "aarch64" => Some(Arch::Arm64),
            "x86_64" => Some(Arch::X64),
            _ => None,
        }
    }
}

impl Asset {
    pub const ALL: [Asset; 8] = [
        Asset::MacZip,
        Asset::MacDmg,
        Asset::MasPkg,
        Asset::WindowsInstaller,
        Asset::WindowsUpdateInstaller,
        Asset::AppImage,
        Asset::Deb,
        Asset::Snap,
    ];

    /// What follows the version: the platform, the architecture as that platform names it, and
    /// the rest of the name.
    fn parts(self, arch: Arch) -> (&'static str, &'static str, &'static str) {
        let arm = arch == Arch::Arm64;
        match self {
            Asset::MacZip => ("mac", if arm { "arm64" } else { "x64" }, ".zip"),
            Asset::MacDmg => ("mac", if arm { "arm64" } else { "x64" }, ".dmg"),
            Asset::MasPkg => ("mac", if arm { "arm64" } else { "x64" }, ".pkg"),
            Asset::WindowsInstaller => ("win", if arm { "arm64" } else { "x64" }, ".exe"),
            Asset::WindowsUpdateInstaller => {
                ("win", if arm { "arm64" } else { "x64" }, "-update.exe")
            }
            Asset::AppImage => ("linux", if arm { "arm64" } else { "x86_64" }, ".AppImage"),
            Asset::Deb => ("linux", if arm { "arm64" } else { "amd64" }, ".deb"),
            Asset::Snap => ("linux", if arm { "arm64" } else { "amd64" }, ".snap"),
        }
    }

    /// The package of release `version` for `arch`.
    pub fn name(self, version: &str, arch: Arch) -> String {
        let (platform, arch, suffix) = self.parts(arch);
        format!("{PREFIX}-{version}-{platform}-{arch}{suffix}")
    }

    /// The architecture KChess releases this kind for: Apple Silicon on macOS, x64 elsewhere.
    pub fn released_arch(self) -> Arch {
        match self {
            Asset::MacZip | Asset::MacDmg | Asset::MasPkg => Arch::Arm64,
            _ => Arch::X64,
        }
    }

    /// The package of release `version` as KChess releases it.
    pub fn released(self, version: &str) -> String {
        self.name(version, self.released_arch())
    }

    /// The kind, version and architecture a package's name gives; none for any other file.
    pub fn parse(name: &str) -> Option<(Asset, &str, Arch)> {
        let rest = name.strip_prefix(PREFIX)?.strip_prefix('-')?;
        Asset::ALL.into_iter().find_map(|asset| {
            [Arch::Arm64, Arch::X64].into_iter().find_map(|arch| {
                let (platform, arch_name, suffix) = asset.parts(arch);
                let version = rest
                    .strip_suffix(suffix)?
                    .strip_suffix(arch_name)?
                    .strip_suffix('-')?
                    .strip_suffix(platform)?
                    .strip_suffix('-')?;
                let plain = !version.is_empty() && !version.contains('-');
                plain.then_some((asset, version, arch))
            })
        })
    }

    /// Whether installations update from this kind, so it gets update patches.
    pub fn patched(self) -> bool {
        matches!(
            self,
            Asset::MacZip | Asset::WindowsInstaller | Asset::AppImage | Asset::Deb
        )
    }

    /// The update patch that makes this package of release `version` from release `from`'s
    /// installed files (`KChess-2026.11.5-linux-amd64.from-2026.11.4.delta`); none for a kind no
    /// installation updates from.
    pub fn patch(self, version: &str, arch: Arch, from: &str) -> Option<String> {
        let (platform, arch, _) = self.parts(arch);
        self.patched()
            .then(|| format!("{PREFIX}-{version}-{platform}-{arch}.from-{from}.delta"))
    }
}
