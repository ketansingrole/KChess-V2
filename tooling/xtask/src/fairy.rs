//! Fairy-Stockfish, the engine that analyses Lichess's variants (Crazyhouse, Atomic, Antichess,
//! Horde, King of the Hill, Racing Kings, Three-check), built from its source at a pinned commit.
//! Its last binary release (2021) has x86-64 builds only, none for macOS, so KChess compiles it:
//! the source archive is checked against its SHA-256, cached in `target/fairy-stockfish`, and
//! built with `make` (MinGW on Windows, linked statically). It is built without a neural network
//! (`nnue=no`, the Makefile's default), so the build downloads nothing else. The program lands in
//! `target/fairy-stockfish/bin`, where a development build of the app finds it.
//!
//! The Makefile compiles for macOS 10.14, which the SDK's libc++ no longer supports (it warns on
//! every file); the build targets the app's own minimum (`package::MACOS_MINIMUM`) instead.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{Result, download, pinned, root, run_with, say};

use crate::pinned::FAIRY_STOCKFISH;

/// The commit built (`pinned.rs`, with the SHA-256 of GitHub's archive of it).
const COMMIT: &str = FAIRY_STOCKFISH.version;

/// The Makefile's `ARCH` for this machine: Apple Silicon, or 64-bit x86 with SSE4.1 and POPCNT.
fn arch() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("apple-silicon"),
        (_, "x86_64") => Ok("x86-64-modern"),
        (os, arch) => Err(format!("No Fairy-Stockfish build for {os}-{arch}.")),
    }
}

/// The Makefile's flag naming the oldest macOS to build for (used only on macOS).
const MACOS_FLAG: &str = "-mmacosx-version-min=";

/// The Makefile with every macOS minimum it names set to `minimum`; an error when it names none,
/// so a new commit that sets the target another way is noticed rather than built for 10.14.
fn target_macos(makefile: &str, minimum: &str) -> Result<String> {
    let mut parts = makefile.split(MACOS_FLAG);
    let mut out = parts.next().unwrap_or_default().to_string();
    let mut found = false;
    for part in parts {
        found = true;
        let version = part.find(|c: char| c.is_whitespace()).unwrap_or(part.len());
        out.push_str(MACOS_FLAG);
        out.push_str(minimum);
        out.push_str(&part[version..]);
    }
    if found {
        Ok(out)
    } else {
        Err(format!("Fairy-Stockfish's Makefile has no {MACOS_FLAG}."))
    }
}

/// What the stamp records: the commit and the macOS it was built for, so either changing rebuilds.
fn stamp_of(commit: &str) -> String {
    format!("{commit} macOS {}", crate::package::MACOS_MINIMUM)
}

/// Build the pinned commit unless it is staged already; the staged program's path.
pub fn build() -> Result<PathBuf> {
    build_with(&Source {
        dir: root().join("target/fairy-stockfish"),
        commit: COMMIT,
        sha256: FAIRY_STOCKFISH.sha256,
        fetch: download,
    })
}

/// The commit to build with its archive's SHA-256, the folder it is cached, built and staged in,
/// and how the archive is fetched.
struct Source<F> {
    dir: PathBuf,
    commit: &'static str,
    sha256: &'static str,
    fetch: F,
}

fn build_with(source: &Source<impl Fn(&str, &Path) -> Result>) -> Result<PathBuf> {
    let Source {
        dir,
        commit,
        sha256,
        fetch,
    } = source;
    let stamp = dir.join("bin/COMMIT");
    let target = dir.join("bin").join(kchess_delta::installed::program(
        kchess_delta::installed::FAIRY_STOCKFISH,
    ));
    if target.exists()
        && std::fs::read_to_string(&stamp).is_ok_and(|built| built.trim() == stamp_of(commit))
    {
        return Ok(target);
    }
    let cache = dir.join(format!("{commit}.tar.gz"));
    let url = crate::pinned::Pin {
        version: commit,
        ..FAIRY_STOCKFISH
    }
    .url();
    let what = format!("Fairy-Stockfish {}", &commit[..12]);
    pinned(&what, &url, sha256, &cache, fetch)?;
    let work = dir.join("work");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|cause| cause.to_string())?;
    run_with(
        Command::new("tar")
            .arg("-xzf")
            .arg(&cache)
            .arg("-C")
            .arg(&work),
    )?;
    let src = work.join(format!("Fairy-Stockfish-{commit}/src"));
    let makefile = src.join("Makefile");
    let text = std::fs::read_to_string(&makefile).map_err(|cause| cause.to_string())?;
    std::fs::write(
        &makefile,
        target_macos(&text, crate::package::MACOS_MINIMUM)?,
    )
    .map_err(|cause| cause.to_string())?;
    say(&format!("Building Fairy-Stockfish ({})…", arch()?));
    let jobs = std::thread::available_parallelism().map_or(2, usize::from);
    // `run_with` runs in the repository; the Makefile is in `src`.
    let arguments = format!("-j{jobs} build ARCH={} -C", arch()?);
    if cfg!(windows) {
        // The Makefile calls `uname` and POSIX shell commands: Git's Bash has them, and MinGW's
        // g++ and make (on the Windows runners' PATH) build a program without MinGW DLLs.
        // `canonicalize` gives a verbatim path (`\\?\D:\…`), which `make -C` cannot open.
        let src = src.to_string_lossy();
        let src = src.strip_prefix(r"\\?\").unwrap_or(&src).replace('\\', "/");
        run_with(
            Command::new(git_bash())
                .arg("-c")
                .arg(format!("mingw32-make {arguments} '{src}' COMP=mingw")),
        )?;
    } else {
        let mut make = Command::new("make");
        make.args(arguments.split(' ')).arg(&src);
        run_with(&mut make)?;
    }
    let built = src.join(kchess_delta::installed::program("stockfish"));
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|cause| cause.to_string())?;
    }
    std::fs::copy(&built, &target)
        .map_err(|cause| format!("Make did not produce {}: {cause}", built.display()))?;
    std::fs::copy(
        work.join(format!("Fairy-Stockfish-{commit}/Copying.txt")),
        target.with_file_name("Copying.txt"),
    )
    .map_err(|cause| format!("Fairy-Stockfish's license is missing: {cause}"))?;
    std::fs::write(&stamp, stamp_of(commit)).map_err(|cause| cause.to_string())?;
    let _ = std::fs::remove_dir_all(&work);
    say(&format!("Built Fairy-Stockfish in {}.", target.display()));
    Ok(target)
}

/// Git's Bash. A plain `bash` on a Windows PATH may be `System32\bash.exe`, which starts WSL.
fn git_bash() -> PathBuf {
    let exec_path = Command::new("git")
        .arg("--exec-path")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok());
    // `<Git>/mingw64/libexec/git-core`: Bash is `<Git>/bin/bash.exe`.
    exec_path
        .as_deref()
        .map(str::trim)
        .and_then(|path| {
            PathBuf::from(path)
                .ancestors()
                .nth(3)
                .map(|git| git.join("bin/bash.exe"))
        })
        .filter(|bash| bash.is_file())
        .unwrap_or_else(|| PathBuf::from("bash"))
}

// The Windows build runs Git's Bash and MinGW's make.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// The build fetches the pinned archive into its folder, unpacks it, runs `make` in its
    /// `src` and stages the program with its license and a stamp of the commit; once staged, it
    /// is kept without unpacking or building again. A stand-in archive's Makefile "builds" a
    /// program naming the `ARCH` it was given.
    #[test]
    fn the_pinned_source_is_fetched_built_and_staged_once() {
        const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
        let scratch = std::env::temp_dir().join(format!("xtask-fairy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let top = format!("Fairy-Stockfish-{COMMIT}");
        let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (name, text) in [
            (
                format!("{top}/src/Makefile"),
                "FLAGS = -mmacosx-version-min=10.14\nbuild:\n\tprintf 'stockfish for $(ARCH) $(FLAGS)' > stockfish\n",
            ),
            (format!("{top}/Copying.txt"), "GPL"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(text.len() as u64);
            header.set_mode(0o644);
            archive
                .append_data(&mut header, name, text.as_bytes())
                .unwrap();
        }
        let archive = archive.into_inner().unwrap().finish().unwrap();
        let fetched = std::cell::RefCell::new(Vec::new());
        let source = Source {
            dir: scratch.clone(),
            commit: COMMIT,
            sha256: kchess_delta::digest(&archive).leak(),
            fetch: |url: &str, to: &Path| -> Result {
                fetched.borrow_mut().push(url.to_string());
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                std::fs::write(to, &archive).map_err(|cause| cause.to_string())
            },
        };

        let target = build_with(&source).unwrap();

        assert_eq!(
            target,
            scratch.join("bin").join(kchess_delta::installed::program(
                kchess_delta::installed::FAIRY_STOCKFISH
            ))
        );
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            format!(
                "stockfish for {} -mmacosx-version-min={}",
                arch().unwrap(),
                crate::package::MACOS_MINIMUM
            )
        );
        assert_eq!(
            std::fs::read_to_string(scratch.join("bin/Copying.txt")).unwrap(),
            "GPL"
        );
        assert_eq!(
            std::fs::read_to_string(scratch.join("bin/COMMIT")).unwrap(),
            format!("{COMMIT} macOS {}", crate::package::MACOS_MINIMUM)
        );
        assert!(!scratch.join("work").exists(), "the work folder was kept");
        assert_eq!(
            *fetched.borrow(),
            [format!(
                "https://codeload.github.com/fairy-stockfish/Fairy-Stockfish/tar.gz/{COMMIT}"
            )]
        );

        // Staged with its stamp: kept, even without the archive to build it from.
        std::fs::remove_file(scratch.join(format!("{COMMIT}.tar.gz"))).unwrap();
        std::fs::write(&target, "kept").unwrap();
        assert_eq!(build_with(&source).unwrap(), target);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "kept");
        assert_eq!(
            fetched.borrow().len(),
            1,
            "a staged build was fetched again"
        );

        // Another commit's stamp, the same commit built for another macOS (a stamp from before
        // the target was set), or no program: built again.
        for stale in ["another", COMMIT, "program"] {
            if stale != "program" {
                std::fs::write(scratch.join("bin/COMMIT"), stale).unwrap();
            } else {
                std::fs::remove_file(&target).unwrap();
            }
            build_with(&source).unwrap();
            assert!(
                std::fs::read_to_string(&target)
                    .unwrap()
                    .starts_with("stockfish for ")
            );
        }
        assert_eq!(fetched.borrow().len(), 2);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Every macOS minimum the Makefile names becomes the app's, and the rest is kept; a Makefile
    /// naming none is refused rather than built for whatever it chooses.
    #[test]
    fn the_makefile_targets_the_apps_macos() {
        let makefile = "a\n\tCXXFLAGS += -arch $(arch) -mmacosx-version-min=10.14\n\
                        \tLDFLAGS += -mmacosx-version-min=10.14 -x\nend -mmacosx-version-min=10.9";
        assert_eq!(
            target_macos(makefile, "12.0").unwrap(),
            "a\n\tCXXFLAGS += -arch $(arch) -mmacosx-version-min=12.0\n\
             \tLDFLAGS += -mmacosx-version-min=12.0 -x\nend -mmacosx-version-min=12.0"
        );
        assert!(target_macos("CXXFLAGS += -O3\n", "12.0").is_err());
    }
}
