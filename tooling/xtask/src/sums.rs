//! `SHA256SUMS.txt`, in `sha256sum`'s format (`<hex>  <name>` a line): what packaging writes
//! beside the packages, the release writes over all its assets (`cargo xtask sums`), the release
//! key signs, and installed apps check every download against.

use std::path::Path;

use crate::util::{Result, say};

/// The file's name.
pub const FILE: &str = "SHA256SUMS.txt";

/// The SHA-256 of the file at `path`, in lowercase hex.
pub fn file(path: &Path) -> Result<String> {
    kchess_delta::file_digest(path).map_err(|cause| format!("{}: {cause}", path.display()))
}

/// Write `dir/SHA256SUMS.txt` listing every other file in `dir`, by name; its text.
pub fn write(dir: &Path) -> Result<String> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|cause| format!("{}: {cause}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            name != FILE && name != format!("{FILE}.sig")
        })
        .collect();
    files.sort();
    let mut sums = String::new();
    for path in &files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        sums.push_str(&format!("{}  {name}\n", file(path)?));
    }
    std::fs::write(dir.join(FILE), &sums).map_err(|cause| cause.to_string())?;
    Ok(sums)
}

/// The digests `sums` lists for `name`: one, unless the file is missing or listed again.
pub fn listed<'a>(sums: &'a str, name: &str) -> Vec<&'a str> {
    sums.lines()
        .filter_map(|line| line.split_once("  "))
        .filter(|(_, listed)| *listed == name)
        .map(|(digest, _)| digest)
        .collect()
}

/// `sums <dir>`: write the release's checksums (release CI, before they are signed).
pub fn run(dir: &Path) -> Result {
    let sums = write(dir)?;
    say(&format!(
        "{} lists {} files.",
        dir.join(FILE).display(),
        sums.lines().count()
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    /// The sums list every file once, as `sha256sum *` does, and never themselves or their
    /// signature, so writing them again gives the same text.
    #[test]
    fn the_sums_list_every_other_file_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.deb"), "hello").unwrap();
        std::fs::write(dir.path().join("a.exe"), "").unwrap();
        std::fs::write(dir.path().join("SHA256SUMS.txt.sig"), "sig").unwrap();
        std::fs::create_dir(dir.path().join("folder")).unwrap();
        let sums = write(dir.path()).unwrap();
        let empty = kchess_delta::digest(b"");
        assert_eq!(sums, format!("{empty}  a.exe\n{HELLO}  b.deb\n"));
        assert_eq!(write(dir.path()).unwrap(), sums);
        std::fs::remove_file(dir.path().join(FILE)).unwrap();
        run(dir.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join(FILE)).unwrap(),
            sums
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(FILE)).unwrap(),
            sums
        );
        assert_eq!(listed(&sums, "b.deb"), [HELLO]);
        assert!(listed(&sums, "b").is_empty());
        assert_eq!(
            listed(&format!("{sums}{HELLO}  a.exe\n"), "a.exe"),
            [empty.as_str(), HELLO]
        );
    }
}
