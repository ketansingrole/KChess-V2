//! The voice runtime the core runs for speech recognition (`crates/kchess-voice`): the helper,
//! the official prebuilt libvosk beside it and their notices, staged in `target/voice-runtime`
//! for development and packaging (the app looks there in a build of this repository).
//!
//! libvosk comes from the Vosk project's own release archives, pinned by SHA-256 and cached in
//! `target/vosk`. A digest mismatch stops the build. The newest prebuilt libraries are 0.3.45
//! (Linux and Windows, Dec 2022) and 0.3.42 (macOS, May 2022).

use std::io::Read;
use std::path::PathBuf;

use crate::util::{Result, root, run, say, sha256_file};

const RELEASES: &str = "https://github.com/alphacep/vosk-api/releases/download";

struct Pin {
    archive: &'static str,
    sha256: &'static str,
    /// Archive member → staged name.
    files: &'static [(&'static str, &'static str)],
}

fn pin() -> Result<Pin> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => Pin {
            archive: "v0.3.42/vosk-osx-0.3.42.zip",
            sha256: "65395f196c9d0583d79949142b25560acaf9c295f36284e18433097f3adb0ea1",
            files: &[("vosk-osx-0.3.42/libvosk.dylib", "libvosk.dylib")],
        },
        ("linux", "x86_64") => Pin {
            archive: "v0.3.45/vosk-linux-x86_64-0.3.45.zip",
            sha256: "bbdc8ed85c43979f6443142889770ea95cbfbc56cffb5c5dcd73afa875c5fbb2",
            files: &[("vosk-linux-x86_64-0.3.45/libvosk.so", "libvosk.so")],
        },
        // libvosk is built with MinGW and needs its C++ runtime beside it.
        ("windows", "x86_64") => Pin {
            archive: "v0.3.45/vosk-win64-0.3.45.zip",
            sha256: "f1dcc9cca460630f81ea8f71794f69c80bed6556d2a4e6237b5785e1d2dff34b",
            files: &[
                ("vosk-win64-0.3.45/libvosk.dll", "libvosk.dll"),
                ("vosk-win64-0.3.45/libstdc++-6.dll", "libstdc++-6.dll"),
                ("vosk-win64-0.3.45/libgcc_s_seh-1.dll", "libgcc_s_seh-1.dll"),
                (
                    "vosk-win64-0.3.45/libwinpthread-1.dll",
                    "libwinpthread-1.dll",
                ),
            ],
        },
        (os, arch) => return Err(format!("No voice runtime for {os}-{arch}.")),
    })
}

/// Where the runtime is staged.
pub fn staged() -> PathBuf {
    root().join("target/voice-runtime")
}

/// Stage the helper built for `profile` (`release` or `debug`) with libvosk and the notices.
pub fn stage(profile: &str) -> Result<PathBuf> {
    let pin = pin()?;
    let name = pin.archive.rsplit('/').next().unwrap_or(pin.archive);
    let cache = root().join("target/vosk").join(name);
    if !cache.exists() || sha256_file(&cache)? != pin.sha256 {
        say(&format!("Downloading {RELEASES}/{}…", pin.archive));
        crate::util::download(&format!("{RELEASES}/{}", pin.archive), &cache)?;
        if sha256_file(&cache)? != pin.sha256 {
            let _ = std::fs::remove_file(&cache);
            return Err(format!(
                "Voice runtime checksum mismatch for {}.",
                pin.archive
            ));
        }
    }
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(&cache).map_err(|cause| cause.to_string())?)
            .map_err(|cause| cause.to_string())?;
    let target = staged();
    let _ = std::fs::remove_dir_all(&target);
    std::fs::create_dir_all(&target).map_err(|cause| cause.to_string())?;
    let executable = format!("kchess-voice{}", std::env::consts::EXE_SUFFIX);
    let helper = root().join("target").join(profile).join(&executable);
    if !helper.exists() {
        return Err(format!(
            "Cargo did not produce {}: build kchess-voice first.",
            helper.display()
        ));
    }
    std::fs::copy(&helper, target.join(&executable)).map_err(|cause| cause.to_string())?;
    for (member, staged_name) in pin.files {
        let mut bytes = Vec::new();
        archive
            .by_name(member)
            .map_err(|_| format!("The archive lacks {member}."))?
            .read_to_end(&mut bytes)
            .map_err(|cause| cause.to_string())?;
        let path = target.join(staged_name);
        std::fs::write(&path, bytes).map_err(|cause| cause.to_string())?;
        // The macOS library is universal; KChess ships only Apple Silicon.
        if cfg!(target_os = "macos") {
            let path = path.to_string_lossy().to_string();
            run("lipo", &[&path, "-thin", "arm64", "-output", &path])?;
        }
    }
    let licenses = root().join("crates/kchess-voice/licenses");
    for entry in std::fs::read_dir(&licenses)
        .map_err(|cause| cause.to_string())?
        .flatten()
    {
        std::fs::copy(entry.path(), target.join(entry.file_name()))
            .map_err(|cause| cause.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in
            std::iter::once(executable.as_str()).chain(pin.files.iter().map(|(_, name)| *name))
        {
            std::fs::set_permissions(target.join(name), std::fs::Permissions::from_mode(0o755))
                .map_err(|cause| cause.to_string())?;
        }
    }
    say(&format!(
        "Staged the voice runtime in target/voice-runtime ({profile})."
    ));
    Ok(target)
}
