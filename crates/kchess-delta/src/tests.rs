use std::path::{Path, PathBuf};

use super::{
    Arch, Asset, Delta, Entry, Error, MAX_HEADER, Node, Source, Tree, UPDATE_EXE,
    WINDOWS_INSTALLER, build, digest, file_digest, name, stage_windows,
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kchess-delta-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn file(data: &[u8], executable: bool) -> Node {
    Node::File {
        data: data.to_vec(),
        executable,
    }
}

/// Bytes that compress poorly, as a program does: a patch of a small change stays small.
fn program(seed: u32, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(2_654_435_761).max(1);
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

/// Write `tree` under `root` as an installation would hold it.
fn install(tree: &Tree, root: &Path) {
    for (path, node) in tree.iter() {
        let at = root.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        match node {
            Node::File { data, .. } => std::fs::write(&at, data).unwrap(),
            Node::Dir => std::fs::create_dir_all(&at).unwrap(),
            Node::Link(target) => std::os::unix::fs::symlink(target, &at).unwrap(),
        }
    }
}

fn delta_file(dir: &Path, old: &Tree, new: &Tree, extras: &[(String, Vec<u8>)]) -> PathBuf {
    let path = dir.join("update.delta");
    let mut out = std::fs::File::create(&path).unwrap();
    build("1.0.0", "1.0.1", old, new, extras, &mut out).unwrap();
    path
}

struct Release {
    old: Tree,
    new: Tree,
    engine: Vec<u8>,
}

/// An installed release and the next: the app rebuilt with a small change, the engine moved
/// to another folder unchanged, a new notice, a changed text and a library dropped.
fn releases() -> Release {
    let engine = program(1, 200_000);
    let app = program(2, 150_000);
    let mut changed_app = app.clone();
    changed_app[70_000..70_016].copy_from_slice(b"a changed string");
    let mut old = Tree::new();
    old.insert("bin/app", file(&app, true));
    old.insert("bin/engine", file(&engine, true));
    old.insert("lib/old.so", file(&program(3, 5_000), false));
    old.insert("readme.txt", file(b"version one", false));
    let mut new = Tree::new();
    new.insert("bin/app", file(&changed_app, true));
    new.insert("engines/engine", file(&engine, true));
    new.insert("readme.txt", file(b"version two, with more to say", false));
    new.insert("notice.txt", file(b"a new file", false));
    new.insert("empty", Node::Dir);
    new.insert("current", Node::Link("bin/app".into()));
    Release { old, new, engine }
}

#[test]
fn a_delta_rebuilds_the_new_release_from_the_installed_one() {
    let dir = scratch("rebuild");
    let release = releases();
    let installed = dir.join("installed");
    install(&release.old, &installed);
    let extras = vec![("control.tar.gz".to_string(), b"control".to_vec())];
    let path = delta_file(&dir, &release.old, &release.new, &extras);

    let mut out = Vec::new();
    let stats = build(
        "1.0.0",
        "1.0.1",
        &release.old,
        &release.new,
        &extras,
        &mut out,
    )
    .unwrap();
    let files: u64 = release
        .new
        .iter()
        .map(|(_, node)| match node {
            Node::File { data, .. } => data.len() as u64,
            _ => 0,
        })
        .sum();
    assert_eq!(stats.files_size, files);
    assert_eq!(stats.copied, 1, "the moved engine");

    let mut delta = Delta::open(&path).unwrap();
    assert_eq!(
        (delta.header.from.as_str(), delta.header.to.as_str()),
        ("1.0.0", "1.0.1")
    );
    let source = |name: &str| {
        delta
            .header
            .entries
            .iter()
            .find_map(|entry| match entry {
                Entry::File { path, source, .. } if path == name => Some(source.clone()),
                _ => None,
            })
            .unwrap()
    };
    // The engine is found by its content, wherever it moved.
    assert!(matches!(source("engines/engine"), Source::Copy(base) if base.path == "bin/engine"));
    assert!(matches!(source("bin/app"), Source::Patch { .. }));
    assert!(matches!(source("notice.txt"), Source::Whole { .. }));

    let out = dir.join("out");
    delta.rebuild(|path| installed.join(path), &out).unwrap();
    assert_eq!(Tree::read(&out, |_| false).unwrap(), release.new);
    assert_eq!(
        std::fs::read(out.join("engines/engine")).unwrap(),
        release.engine
    );
    assert!(
        !out.join("lib/old.so").exists(),
        "a dropped file is not carried over"
    );
    assert_eq!(delta.extra("control.tar.gz").unwrap(), b"control");
    assert!(delta.extra("missing").is_err());

    // The patch is a small part of what it makes.
    let size = std::fs::metadata(&path).unwrap().len();
    assert!(size < 30_000, "{size} bytes");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A Windows patch is staged as the update installer reads it: the files in `files`, the
/// installer beside them.
#[test]
fn a_windows_patch_stages_the_files_beside_the_update_installer() {
    let dir = scratch("windows");
    let release = releases();
    let installed = dir.join("installed");
    install(&release.old, &installed);
    let extras = vec![(WINDOWS_INSTALLER.to_string(), b"installer".to_vec())];
    let mut delta = Delta::open(&delta_file(&dir, &release.old, &release.new, &extras)).unwrap();
    let out = dir.join("staged");
    let installer = stage_windows(&mut delta, &installed, &out).unwrap();
    assert_eq!(installer, out.join(UPDATE_EXE));
    assert_eq!(std::fs::read(&installer).unwrap(), b"installer");
    assert_eq!(
        Tree::read(&out.join("files"), |_| false).unwrap(),
        release.new
    );
    // Without the installer, nothing would run the files.
    let mut bare = Delta::open(&delta_file(&dir, &release.old, &release.new, &[])).unwrap();
    assert!(stage_windows(&mut bare, &installed, &dir.join("bare")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Nothing about which file changes is assumed: a release that replaces the engine carries the
/// new one, and the stats say what each file became.
#[test]
fn a_changed_engine_is_carried_as_any_file() {
    let dir = scratch("engine");
    let release = releases();
    let mut new = release.new.clone();
    new.insert("engines/engine", file(&program(9, 120_000), true));
    let mut out = Vec::new();
    let stats = build("1.0.0", "1.0.1", &release.old, &new, &[], &mut out).unwrap();
    assert_eq!((stats.copied, stats.patched, stats.whole), (0, 1, 3));
    assert_eq!(stats.delta_size, out.len() as u64);
    assert!(stats.delta_size > 120_000, "the new engine is in the delta");

    let installed = dir.join("installed");
    install(&release.old, &installed);
    std::fs::write(dir.join("update.delta"), &out).unwrap();
    let mut delta = Delta::open(&dir.join("update.delta")).unwrap();
    delta
        .rebuild(|path| installed.join(path), &dir.join("out"))
        .unwrap();
    assert_eq!(Tree::read(&dir.join("out"), |_| false).unwrap(), new);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A user who deleted the engine, or an installation changed after it was installed, cannot
/// be patched: the updater then downloads the whole package.
#[test]
fn a_missing_or_changed_base_is_refused() {
    let dir = scratch("bases");
    let release = releases();
    let path = delta_file(&dir, &release.old, &release.new, &[]);
    let installed = dir.join("installed");
    install(&release.old, &installed);
    let mut delta = Delta::open(&path).unwrap();
    assert_eq!(delta.check_bases(|path| installed.join(path)), Ok(()));
    assert_eq!(delta.bases().len(), 2);

    std::fs::remove_file(installed.join("bin/engine")).unwrap();
    let refused = delta.rebuild(|path| installed.join(path), &dir.join("gone"));
    assert_eq!(refused.unwrap_err().0, "bin/engine is missing");
    assert!(
        !dir.join("gone").exists(),
        "nothing is written before the bases are checked"
    );

    install(&release.old, &installed);
    std::fs::write(installed.join("bin/app"), b"changed by hand").unwrap();
    let refused = delta.rebuild(|path| installed.join(path), &dir.join("changed"));
    assert_eq!(refused.unwrap_err().0, "bin/app was changed");
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the delta makes is checked file by file, so a damaged patch never stages an update.
#[test]
fn a_damaged_delta_is_refused() {
    let dir = scratch("damaged");
    let release = releases();
    let path = delta_file(&dir, &release.old, &release.new, &[]);
    let installed = dir.join("installed");
    install(&release.old, &installed);
    let bytes = std::fs::read(&path).unwrap();

    let open = |bytes: &[u8]| {
        std::fs::write(&path, bytes).unwrap();
        Delta::open(&path)
    };
    assert_eq!(
        open(b"not a delta at all").err().unwrap().0,
        "The update patch is not one KChess reads"
    );
    assert!(open(&bytes[..bytes.len() - 10]).is_err(), "cut short");
    let mut long = bytes.clone();
    long[13..21].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(open(&long).is_err());

    // The header names its blobs; a file whose digest does not match what it makes is refused.
    let mut delta = open(&bytes).unwrap();
    for entry in &mut delta.header.entries {
        if let Entry::File { path, sha256, .. } = entry
            && path == "notice.txt"
        {
            *sha256 = digest(b"something else");
        }
    }
    let refused = delta.rebuild(|path| installed.join(path), &dir.join("out"));
    assert_eq!(
        refused.unwrap_err().0,
        "notice.txt did not match its checksum"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_delta_never_writes_outside_its_folder() {
    let dir = scratch("outside");
    for bad in [
        "../escape",
        "/etc/passwd",
        "a/../../b",
        "",
        "c:/x",
        "a\\b",
        "a//b",
        "./a",
    ] {
        let mut new = Tree::new();
        new.insert(bad, file(b"x", false));
        let path = delta_file(&dir, &Tree::new(), &new, &[]);
        let refused = Delta::open(&path).err().unwrap().0;
        assert!(refused.contains("outside the app"), "{bad}: {refused}");
    }
    let mut new = Tree::new();
    new.insert("format", file(b"x", false));
    let path = delta_file(&dir, &Tree::new(), &new, &[]);
    let mut text = std::fs::read(&path).unwrap();
    let at = text.windows(10).position(|w| w == b"\"format\":1").unwrap();
    text[at + 9] = b'7';
    std::fs::write(&path, &text).unwrap();
    assert_eq!(
        Delta::open(&path).err().unwrap().0,
        "The update patch is format 7, not 1"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tree_reads_a_folder_as_installed() {
    let dir = scratch("tree");
    let release = releases();
    install(&release.new, &dir);
    std::fs::write(dir.join("Uninstall KChess.exe"), b"made by the installer").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.join("bin/app"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(
        dir.join("engines/engine"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let read = Tree::read(&dir, |path| path.starts_with("Uninstall ")).unwrap();
    assert_eq!(read, release.new);
    assert_eq!(read.len(), 6);
    assert!(!read.is_empty() && Tree::new().is_empty());

    let mut bundle = Tree::new();
    bundle.insert("KChess.app/Contents/MacOS/kchess", file(b"app", true));
    bundle.insert("KChess.app/Contents/Info.plist", file(b"plist", false));
    bundle.insert("KChess.app", Node::Dir);
    bundle.insert("Other/file", file(b"other", false));
    let inner = bundle.under("KChess.app");
    assert_eq!(
        inner.iter().map(|(path, _)| path).collect::<Vec<_>>(),
        ["Contents/Info.plist", "Contents/MacOS/kchess"]
    );
    assert_eq!(
        file_digest(&dir.join("readme.txt")).unwrap(),
        digest(b"version two, with more to say")
    );
    assert!(file_digest(&dir.join("missing")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_delta_is_named_after_its_package_and_base() {
    assert_eq!(
        name("KChess-2026.11.5-linux-amd64.deb", "2026.11.4").as_deref(),
        Some("KChess-2026.11.5-linux-amd64.from-2026.11.4.delta")
    );
    assert_eq!(
        name("KChess-2026.11.5-linux-x86_64.AppImage", "2026.11.3").as_deref(),
        Some("KChess-2026.11.5-linux-x86_64.from-2026.11.3.delta")
    );
    assert_eq!(
        name("KChess-2026.11.5-mac-arm64.zip", "2026.11.4").as_deref(),
        Some("KChess-2026.11.5-mac-arm64.from-2026.11.4.delta")
    );
    assert_eq!(
        name("KChess-2026.11.5-win-x64.exe", "2026.11.4").as_deref(),
        Some("KChess-2026.11.5-win-x64.from-2026.11.4.delta")
    );
    assert_eq!(name("KChess-2026.11.5-mac-arm64.dmg", "2026.11.4"), None);
    assert_eq!(
        name("KChess-2026.11.5-win-x64-update.exe", "2026.11.4"),
        None
    );
    assert_eq!(name("notes.exe", "2026.11.4"), None);
}

/// The names installed apps ask releases for: never change them.
#[test]
fn every_package_keeps_the_name_installations_ask_for() {
    let released: Vec<String> = Asset::ALL
        .into_iter()
        .map(|asset| asset.released("2026.11.5"))
        .collect();
    assert_eq!(
        released,
        [
            "KChess-2026.11.5-mac-arm64.zip",
            "KChess-2026.11.5-mac-arm64.dmg",
            "KChess-2026.11.5-mac-arm64.pkg",
            "KChess-2026.11.5-win-x64.exe",
            "KChess-2026.11.5-win-x64-update.exe",
            "KChess-2026.11.5-linux-x86_64.AppImage",
            "KChess-2026.11.5-linux-amd64.deb",
            "KChess-2026.11.5-linux-amd64.snap",
        ]
    );
    assert_eq!(
        Asset::AppImage.name("2026.11.5", Arch::Arm64),
        "KChess-2026.11.5-linux-arm64.AppImage"
    );
    assert_eq!(
        Asset::MacZip.name("2026.11.5", Arch::X64),
        "KChess-2026.11.5-mac-x64.zip"
    );
}

#[test]
fn packages_are_made_for_this_machine() {
    let expected = match std::env::consts::ARCH {
        "aarch64" => Some(Arch::Arm64),
        "x86_64" => Some(Arch::X64),
        _ => None,
    };
    assert_eq!(Arch::current(), expected);
}

#[test]
fn a_package_is_known_by_its_name() {
    for asset in Asset::ALL {
        for arch in [Arch::Arm64, Arch::X64] {
            let name = asset.name("2026.11.5", arch);
            assert_eq!(
                Asset::parse(&name),
                Some((asset, "2026.11.5", arch)),
                "{name}"
            );
        }
    }
    for other in [
        "SHA256SUMS.txt",
        "latest-mac.yml",
        "KChess-2026.11.5-linux-amd64.from-2026.11.4.delta",
        "KChess-Preview-2026.10.1-win-x64.exe",
        "KChess--win-x64.exe",
        "KChess-2026.11.5-win-x86.exe",
        "Other-2026.11.5-win-x64.exe",
    ] {
        assert_eq!(Asset::parse(other), None, "{other}");
    }
}

/// A file replaced by unrelated bytes comes whole: its patch would be no smaller.
#[test]
fn a_file_no_patch_helps_comes_whole() {
    let mut old = Tree::new();
    old.insert("bin/app", file(&program(11, 60_000), true));
    let mut new = Tree::new();
    new.insert("bin/app", file(&program(12, 60_000), true));
    let mut out = Vec::new();
    let stats = build("1.0.0", "1.0.1", &old, &new, &[], &mut out).unwrap();
    assert_eq!((stats.copied, stats.patched, stats.whole), (0, 0, 1));
    let dir = scratch("whole");
    std::fs::write(dir.join("d"), &out).unwrap();
    let delta = Delta::open(&dir.join("d")).unwrap();
    assert!(
        delta.bases().is_empty(),
        "a whole file needs nothing installed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn errors_read_as_their_message() {
    assert_eq!(
        Error("bin/app is missing".into()).to_string(),
        "bin/app is missing"
    );
    let io: Error = std::io::Error::other("disk full").into();
    assert_eq!(io.to_string(), "disk full");
}

/// A header of exactly the longest length is read (and here found cut short), not refused.
#[test]
fn the_longest_header_is_allowed() {
    let dir = scratch("longest");
    let path = dir.join("d");
    let write = |length: u64| {
        let mut bytes = super::MAGIC.to_vec();
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(b"{}");
        std::fs::write(&path, bytes).unwrap();
        Delta::open(&path).err().unwrap().0
    };
    assert_eq!(
        write(MAX_HEADER + 1),
        "The update patch's header is too long"
    );
    assert_ne!(write(MAX_HEADER), "The update patch's header is too long");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An installed file a patch copies is checked before anything is written, like one it patches.
#[test]
fn a_changed_file_to_copy_is_refused() {
    let dir = scratch("copy-base");
    let release = releases();
    let path = delta_file(&dir, &release.old, &release.new, &[]);
    let installed = dir.join("installed");
    install(&release.old, &installed);
    std::fs::write(installed.join("bin/engine"), b"quarantined").unwrap();
    let delta = Delta::open(&path).unwrap();
    assert_eq!(
        delta.check_bases(|path| installed.join(path)),
        Err(Error("bin/engine was changed".into()))
    );
    let _ = std::fs::remove_dir_all(&dir);
}
