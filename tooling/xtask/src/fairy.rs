//! Fairy-Stockfish, the engine that analyses Lichess's variants (Crazyhouse, Atomic, Antichess,
//! Horde, King of the Hill, Racing Kings, Three-check), built from its source at a pinned commit.
//! Its last binary release (2021) has x86-64 builds only, none for macOS, so KChess compiles it:
//! the source archive is checked against its SHA-256, cached in `target/fairy-stockfish`, and
//! built with `make` (MinGW on Windows, linked statically). It is built without a neural network
//! (`nnue=no`, the Makefile's default), so the build downloads nothing else. The program lands in
//! `target/fairy-stockfish/bin`, where a development build of the app finds it.

use std::path::PathBuf;
use std::process::Command;

use crate::util::{Result, root, run_with, say, sha256_file};

/// The commit built, and the SHA-256 of GitHub's archive of it.
const COMMIT: &str = "2f1959e1f0b44d4648ff35015b89d8e918a5c746";
const SHA256: &str = "f226f3c67cb30e87b6990ce7d38cf29cbaca59709f2dba23f421fafdecbb67e5";

/// The program's name, as the app looks for it.
pub fn executable() -> String {
    format!("fairy-stockfish{}", std::env::consts::EXE_SUFFIX)
}

/// Where the built program is staged.
pub fn staged() -> PathBuf {
    root().join("target/fairy-stockfish/bin").join(executable())
}

/// The Makefile's `ARCH` for this machine: Apple Silicon, or 64-bit x86 with SSE4.1 and POPCNT.
fn arch() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("apple-silicon"),
        (_, "x86_64") => Ok("x86-64-modern"),
        (os, arch) => Err(format!("No Fairy-Stockfish build for {os}-{arch}.")),
    }
}

/// Build the pinned commit unless it is staged already; the staged program's path.
pub fn build() -> Result<PathBuf> {
    let stamp = root().join("target/fairy-stockfish/bin/COMMIT");
    let target = staged();
    if target.exists() && std::fs::read_to_string(&stamp).is_ok_and(|built| built.trim() == COMMIT)
    {
        return Ok(target);
    }
    let cache = root().join(format!("target/fairy-stockfish/{COMMIT}.tar.gz"));
    if !cache.exists() || sha256_file(&cache)? != SHA256 {
        let url =
            format!("https://codeload.github.com/fairy-stockfish/Fairy-Stockfish/tar.gz/{COMMIT}");
        say(&format!("Downloading Fairy-Stockfish {}…", &COMMIT[..12]));
        crate::util::download(&url, &cache)?;
        if sha256_file(&cache)? != SHA256 {
            let _ = std::fs::remove_file(&cache);
            return Err(format!(
                "Fairy-Stockfish source checksum mismatch for {COMMIT}."
            ));
        }
    }
    let work = root().join("target/fairy-stockfish/work");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|cause| cause.to_string())?;
    run_with(
        Command::new("tar")
            .arg("-xzf")
            .arg(&cache)
            .arg("-C")
            .arg(&work),
    )?;
    let src = work.join(format!("Fairy-Stockfish-{COMMIT}/src"));
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
    let built = src.join(format!("stockfish{}", std::env::consts::EXE_SUFFIX));
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|cause| cause.to_string())?;
    }
    std::fs::copy(&built, &target)
        .map_err(|cause| format!("Make did not produce {}: {cause}", built.display()))?;
    std::fs::copy(
        work.join(format!("Fairy-Stockfish-{COMMIT}/Copying.txt")),
        target.with_file_name("Copying.txt"),
    )
    .map_err(|cause| format!("Fairy-Stockfish's license is missing: {cause}"))?;
    std::fs::write(&stamp, COMMIT).map_err(|cause| cause.to_string())?;
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
