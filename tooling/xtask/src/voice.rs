//! The voice runtime the core runs for speech recognition (`crates/kchess-voice`): the helper,
//! the official prebuilt libvosk beside it and their notices, staged in `target/voice-runtime`
//! for development and packaging (the app looks there in a build of this repository).
//!
//! libvosk comes from the Vosk project's own release archives, pinned by SHA-256 (`pinned.rs`)
//! and cached in `target/vosk`. A digest mismatch stops the build.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::util::{Result, download, pinned, root, run, say};

use crate::pinned::{self, VOSK_LINUX_X64, VOSK_MACOS, VOSK_WINDOWS_X64};

/// This machine's libvosk archive.
struct Pin {
    archive: pinned::Pin,
    /// Archive member → staged name.
    files: &'static [(&'static str, &'static str)],
}

fn pin() -> Result<Pin> {
    use kchess_voice::LIBRARY;
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", _) => Pin {
            archive: VOSK_MACOS,
            files: &[("vosk-osx-0.3.42/libvosk.dylib", LIBRARY)],
        },
        ("linux", "x86_64") => Pin {
            archive: VOSK_LINUX_X64,
            files: &[("vosk-linux-x86_64-0.3.45/libvosk.so", LIBRARY)],
        },
        // libvosk is built with MinGW and needs its C++ runtime beside it.
        ("windows", "x86_64") => Pin {
            archive: VOSK_WINDOWS_X64,
            files: &[
                ("vosk-win64-0.3.45/libvosk.dll", LIBRARY),
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

pub fn staged() -> PathBuf {
    root()
        .join("target")
        .join(kchess_delta::installed::BUILT_VOICE)
}

/// Stage the helper built for `profile` unless what is staged is already as new. Staging replaces
/// the folder, so a running helper is left alone when nothing changed.
pub fn stage_if_outdated(profile: &str) -> Result<()> {
    use crate::util::modified;
    let executable = kchess_delta::installed::program(kchess_delta::installed::VOICE_HELPER);
    let built = modified(&root().join("target").join(profile).join(&executable));
    if outdated(modified(&staged().join(&executable)), built) {
        stage(profile)?;
    }
    Ok(())
}

/// Whether the helper staged at time `staged` is older than the one built at `built`, or none
/// is staged.
fn outdated(staged: Option<SystemTime>, built: Option<SystemTime>) -> bool {
    staged.is_none_or(|current| Some(current) < built)
}

/// Stage the helper built for `profile` (`release` or `debug`) with libvosk and the notices.
pub fn stage(profile: &str) -> Result<PathBuf> {
    stage_with(
        &Runtime {
            pin: pin()?,
            cache: root().join("target/vosk"),
            built: root().join("target").join(profile),
            licenses: root().join("crates/kchess-voice/licenses"),
            target: staged(),
            fetch: download,
        },
        profile,
    )
}

/// What staging reads and writes: the pinned libvosk archive, the folder it is cached in, the
/// folder Cargo built the helper in, the notices, the staging folder, and how the archive is
/// fetched.
struct Runtime<F> {
    pin: Pin,
    cache: PathBuf,
    built: PathBuf,
    licenses: PathBuf,
    target: PathBuf,
    fetch: F,
}

fn stage_with(runtime: &Runtime<impl Fn(&str, &Path) -> Result>, profile: &str) -> Result<PathBuf> {
    let Runtime {
        pin,
        cache,
        built,
        licenses,
        target,
        fetch,
    } = runtime;
    let cache = cache.join(pin.archive.file);
    let url = pin.archive.url();
    pinned(
        &format!("libvosk ({url})"),
        &url,
        pin.archive.sha256,
        &cache,
        fetch,
    )?;
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(&cache).map_err(|cause| cause.to_string())?)
            .map_err(|cause| cause.to_string())?;
    let _ = std::fs::remove_dir_all(target);
    std::fs::create_dir_all(target).map_err(|cause| cause.to_string())?;
    let executable = kchess_delta::installed::program(kchess_delta::installed::VOICE_HELPER);
    let helper = built.join(&executable);
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
    for entry in std::fs::read_dir(licenses)
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
    Ok(target.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The helper is staged in the build folder, where the app's debug build looks for it.
    #[test]
    fn the_helper_is_staged_in_the_build_folder() {
        assert_eq!(staged(), root().join("target/voice-runtime"));
    }

    /// The helper is staged again only when none is, or the build is newer than what is.
    #[test]
    fn only_an_older_or_missing_helper_is_staged_again() {
        let then = SystemTime::UNIX_EPOCH;
        let later = then + std::time::Duration::from_secs(1);
        assert!(outdated(None, Some(then)));
        assert!(outdated(None, None));
        assert!(outdated(Some(then), Some(later)));
        assert!(!outdated(Some(later), Some(then)));
        assert!(!outdated(Some(then), Some(then)));
        assert!(!outdated(Some(then), None));
    }

    /// A file as the stand-in archive holds it. On macOS staging thins the universal library to
    /// Apple Silicon, so the archive holds `bytes` as the arm64 slice of a fat file.
    fn archived(bytes: &[u8]) -> Vec<u8> {
        if !cfg!(target_os = "macos") {
            return bytes.to_vec();
        }
        const OFFSET: u32 = 1 << 14;
        let mut fat = Vec::new();
        for word in [
            0xcafe_babe,
            1,
            0x0100_000c,
            0,
            OFFSET,
            bytes.len() as u32,
            14,
        ] {
            fat.extend_from_slice(&u32::to_be_bytes(word));
        }
        fat.resize(OFFSET as usize, 0);
        fat.extend_from_slice(bytes);
        fat
    }

    /// Staging fetches the pinned archive into its cache and replaces the staging folder with the
    /// helper Cargo built, the archive's libraries under their staged names and the notices.
    #[test]
    fn the_helper_is_staged_with_the_pinned_libraries_and_the_notices() {
        use std::io::Write;
        let scratch = std::env::temp_dir().join(format!("xtask-voice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in [("vosk-0.1/libvosk", b"vosk"), ("vosk-0.1/runtime", b"libc")] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&archived(bytes)).unwrap();
        }
        let archive = zip.finish().unwrap().into_inner();
        let executable = kchess_delta::installed::program(kchess_delta::installed::VOICE_HELPER);
        let built = scratch.join("built");
        let licenses = scratch.join("licenses");
        for (dir, name, text) in [
            (&built, executable.as_str(), "helper"),
            (&licenses, "NOTICE.txt", "notice"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(name), text).unwrap();
        }
        let target = scratch.join("staged");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("left over"), "").unwrap();
        let fetched = std::cell::RefCell::new(Vec::new());
        let runtime = Runtime {
            pin: Pin {
                archive: pinned::Pin {
                    version: "v0.1",
                    file: "vosk-0.1.zip",
                    sha256: kchess_delta::digest(&archive).leak(),
                    ..VOSK_LINUX_X64
                },
                files: &[
                    ("vosk-0.1/libvosk", "libvosk.so"),
                    ("vosk-0.1/runtime", "libc.so"),
                ],
            },
            cache: scratch.join("cache"),
            built,
            licenses,
            target: target.clone(),
            fetch: |url: &str, to: &Path| -> Result {
                fetched.borrow_mut().push(url.to_string());
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                std::fs::write(to, &archive).map_err(|cause| cause.to_string())
            },
        };

        assert_eq!(stage_with(&runtime, "debug").unwrap(), target);

        assert_eq!(
            *fetched.borrow(),
            ["https://github.com/alphacep/vosk-api/releases/download/v0.1/vosk-0.1.zip"]
        );
        assert!(scratch.join("cache/vosk-0.1.zip").exists());
        let mut staged: Vec<_> = std::fs::read_dir(&target)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        staged.sort();
        let mut expected = vec![
            executable.clone(),
            "NOTICE.txt".into(),
            "libc.so".into(),
            "libvosk.so".into(),
        ];
        expected.sort();
        assert_eq!(staged, expected);
        for (name, text) in [
            (executable.as_str(), "helper"),
            ("libvosk.so", "vosk"),
            ("libc.so", "libc"),
            ("NOTICE.txt", "notice"),
        ] {
            assert_eq!(std::fs::read_to_string(target.join(name)).unwrap(), text);
        }
        #[cfg(unix)]
        for name in [executable.as_str(), "libvosk.so", "libc.so"] {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(target.join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755, "{name}");
        }

        // Without the helper, staging stops and says to build it.
        std::fs::remove_file(runtime.built.join(&executable)).unwrap();
        let error = stage_with(&runtime, "debug").unwrap_err();
        assert!(error.contains("build kchess-voice first"), "{error}");
        assert_eq!(
            fetched.borrow().len(),
            1,
            "a cached archive was fetched again"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
