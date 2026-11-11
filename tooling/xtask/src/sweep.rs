//! Deletes the object files in the build's `deps` folders that no program needs any more.
//!
//! On macOS a debug build keeps each program's compiled objects (`*.rcgu.o`) beside it: its debug
//! info stays in them, and the program's debug map names them (`split-debuginfo = "unpacked"`,
//! Cargo's default there). An incremental build names them afresh every time and leaves the old
//! ones behind, so `target/debug/deps` only grows: 588,561 files (47 GB) by 2026-11-10, and
//! every build step slowed down with it. `check-fast` after a one-line `kchess-core` edit took
//! 213–246 s; with the 503,000 objects no program named deleted, 89–92 s. A library's objects
//! are never needed loose: programs name them inside the library's `.rlib`.
//!
//! It runs after the sweep of builds no current build uses (`superseded.rs`), which goes on every
//! platform.
//!
//! So `check-fast`, `check` and `app` first delete every loose object that is neither named by
//! the debug map of the program or library it was compiled for, nor that program's own when it
//! cannot be read. They skip a folder while a build holds Cargo's lock on it, whose objects may
//! be about to be linked.

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::util::say;

/// A 64-bit Mach-O file's magic number, as its first four bytes read little-endian.
const MH_MAGIC_64: u32 = 0xfeed_facf;
/// The load command that locates the symbol and string tables.
const LC_SYMTAB: u32 = 0x2;
/// A debug-map entry naming an object file.
const N_OSO: u8 = 0x66;
/// The bytes of a 64-bit symbol table entry (`nlist_64`).
const NLIST_64: usize = 16;
/// The file beside a profile's `deps` whose time is the last sweep's.
const SWEPT: &str = ".kchess-swept";
/// The longest object path read from the string table.
const LONGEST_PATH: usize = 4096;

/// Sweeps the build folder: the units no current build uses (`superseded.rs`) on every platform,
/// then on macOS the objects no program uses, saying what went; then the build folder of the
/// checkout's mutants tree, unless a push is testing in it; then the build folders of the other
/// worktrees nobody is building in (`worktrees.rs`). With `dry_run`, deletes nothing and says
/// what would go. Refuses a build afterwards when the disk is still too full for one.
pub fn build_folder(dry_run: bool) -> crate::util::Result {
    let target = target_folder(std::env::var_os("CARGO_TARGET_DIR"));
    let root = crate::util::root();
    let mut messages = sweep_all(&target, &root, dry_run, cfg!(target_os = "macos"));
    messages.extend(idle_mutants_tree(&root, dry_run));
    messages.extend(crate::worktrees::sweep(&root, dry_run));
    for message in messages {
        say(&message);
    }
    if dry_run {
        Ok(())
    } else {
        crate::worktrees::enough_space(crate::worktrees::free_space(&root))
    }
}

/// Sweeps the build folder (`target`) of the pre-push hook's mutants tree (`mutants_tree.rs`),
/// which builds a pushed commit's mutants in place and so replaces units as the checkout's own
/// build does: the units no current build of the tree (its parent) uses, under the same build
/// lock. What to say about it.
pub fn mutants_tree(target: &Path, dry_run: bool) -> Vec<String> {
    sweep_all(target, target.parent().unwrap_or(target), dry_run, false)
        .into_iter()
        .map(|message| format!("Mutants tree: {message}"))
        .collect()
}

/// [`mutants_tree`] of `checkout`'s tree, while no push holds it: one that does sweeps it itself.
fn idle_mutants_tree(checkout: &Path, dry_run: bool) -> Vec<String> {
    crate::mutants_tree::idle_target(checkout)
        .map(|(target, _lock)| mutants_tree(&target, dry_run))
        .unwrap_or_default()
}

/// Sweeps every profile folder in `target`, the build folder of the workspace at `workspace`, and
/// its objects when `objects`: what to say about it.
fn sweep_all(target: &Path, workspace: &Path, dry_run: bool, objects: bool) -> Vec<String> {
    let folders = deps_folders(target);
    let targets = crate::superseded::workspace_targets(workspace);
    let (units, unit_bytes) = folders
        .iter()
        .map(|deps| superseded(deps, targets.as_ref(), dry_run))
        .fold((0, 0), |(units, bytes), (more, size)| {
            (units + more, bytes + size)
        });
    let (files, bytes) = if objects {
        folders
            .iter()
            .map(|deps| sweep(deps, dry_run))
            .fold((0, 0), |(files, bytes), (more, size)| {
                (files + more, bytes + size)
            })
    } else {
        (0, 0)
    };
    [
        units_summary(units, unit_bytes, dry_run),
        summary(files, bytes, dry_run),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// What a sweep of objects says: nothing when it found none.
fn summary(files: usize, bytes: u64, dry_run: bool) -> Option<String> {
    let plural = if files == 1 { "" } else { "s" };
    let verb = if dry_run { "Would delete" } else { "Deleted" };
    (files > 0).then(|| {
        format!(
            "{verb} {files} object file{plural} no program uses ({} MB).",
            bytes / 1_000_000
        )
    })
}

/// What a sweep of units no current build uses says: nothing when it found none.
fn units_summary(units: usize, bytes: u64, dry_run: bool) -> Option<String> {
    let plural = if units == 1 { "" } else { "s" };
    let verb = if dry_run { "Would delete" } else { "Deleted" };
    (units > 0).then(|| {
        format!(
            "{verb} {units} build{plural} no current build uses ({:.1} GB).",
            bytes as f64 / 1e9
        )
    })
}

/// Deletes, under Cargo's build lock, the units of one profile folder (the parent of `deps`)
/// that no current build of the workspace's `targets` uses: how many, and their size.
fn superseded(
    deps: &Path,
    targets: Option<&crate::superseded::Targets>,
    dry_run: bool,
) -> (usize, u64) {
    let Some(profile) = deps.parent() else {
        return (0, 0);
    };
    let Some(_lock) = build_lock(profile) else {
        return (0, 0);
    };
    let plan = crate::superseded::plan(profile, targets);
    if dry_run {
        (plan.units, plan.bytes)
    } else {
        (plan.units, crate::superseded::delete(&plan))
    }
}

/// The build folder: `CARGO_TARGET_DIR` (given), else the repository's `target`.
fn target_folder(cargo_target_dir: Option<std::ffi::OsString>) -> PathBuf {
    cargo_target_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::util::root().join("target"))
}

/// `target/<profile>/deps` and `target/<triple or tool>/<profile>/deps`.
pub(crate) fn deps_folders(target: &Path) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    for first in subfolders(target) {
        if first.join("deps").is_dir() {
            folders.push(first.join("deps"));
        }
        for second in subfolders(&first) {
            if second.file_name().is_some_and(|name| name != "deps") && second.join("deps").is_dir()
            {
                folders.push(second.join("deps"));
            }
        }
    }
    folders.sort();
    folders
}

fn subfolders(folder: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// Deletes one `deps` folder's stale objects under Cargo's build lock: how many, and their size.
/// With `dry_run`, only counts them.
fn sweep(deps: &Path, dry_run: bool) -> (usize, u64) {
    let Some(_lock) = deps.parent().and_then(build_lock) else {
        return (0, 0);
    };
    let names: Vec<String> = std::fs::read_dir(deps)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    // A program's objects change only when it is built again, so one not built since the last
    // sweep keeps them unread: a sweep after a rebuild reads a few programs, not hundreds.
    let marker = deps.with_file_name(SWEPT);
    let since = crate::util::modified(&marker);
    let unchanged = |path: &Path| {
        since.is_some_and(|since| crate::util::modified(path).is_some_and(|built| built <= since))
    };
    let mut deleted = (0, 0);
    let stale = stale(&names, |image| {
        let path = deps.join(image);
        if unchanged(&path) {
            Image::Kept
        } else {
            debug_map_of(&path)
        }
    });
    for name in stale {
        let path = deps.join(&name);
        let size = std::fs::metadata(&path).map_or(0, |file| file.len());
        if dry_run || std::fs::remove_file(&path).is_ok() {
            deleted = (deleted.0 + 1, deleted.1 + size);
        }
    }
    if !dry_run {
        let _ = std::fs::write(&marker, b"");
    }
    deleted
}

/// The lock files Cargo keeps in a profile folder.
pub(crate) const LOCKS: [&str; 3] = [".cargo-build-lock", ".cargo-artifact-lock", ".cargo-lock"];

/// Cargo's locks on a profile folder, held while no build is running there; `None` while one is
/// (or when they cannot be opened), so its objects are left alone.
pub(crate) fn build_lock(profile: &Path) -> Option<Vec<File>> {
    let mut held = Vec::new();
    for name in LOCKS {
        let path = profile.join(name);
        if !path.exists() {
            continue;
        }
        let file = File::options().read(true).write(true).open(&path).ok()?;
        file.try_lock().ok()?;
        held.push(file);
    }
    Some(held)
}

/// What a program's or library's file tells about the objects compiled for it.
#[derive(Debug, PartialEq)]
enum Image {
    /// Not there: its objects are left from a build no longer kept, or are a library's.
    Missing,
    /// There, and not a Mach-O file this reads or not built since the last sweep: its objects
    /// are kept.
    Kept,
    /// The objects its debug map names, by file name.
    Names(HashSet<String>),
}

/// The loose objects among a `deps` folder's file `names` that no program needs, given what each
/// program's or library's file (by name) holds. An object `<crate>-<hash>.<…>.rcgu.o` belongs to
/// the program `<crate>-<hash>` or the library `lib<crate>-<hash>.dylib` beside it.
fn stale(names: &[String], image: impl Fn(&str) -> Image) -> Vec<String> {
    let present: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut images: std::collections::HashMap<&str, Image> = std::collections::HashMap::new();
    let mut stale = Vec::new();
    for name in names {
        let Some((unit, _)) = name
            .strip_suffix(".rcgu.o")
            .and_then(|object| object.split_once('.'))
        else {
            continue;
        };
        let known = images.entry(unit).or_insert_with(|| {
            let dylib = format!("lib{unit}.dylib");
            if present.contains(unit) {
                image(unit)
            } else if present.contains(dylib.as_str()) {
                image(&dylib)
            } else {
                Image::Missing
            }
        });
        let needed = match known {
            Image::Missing => false,
            Image::Kept => true,
            Image::Names(names) => names.contains(name),
        };
        if !needed {
            stale.push(name.clone());
        }
    }
    stale
}

fn debug_map_of(path: &Path) -> Image {
    match File::open(path)
        .ok()
        .and_then(|mut file| debug_map(&mut file))
    {
        Some(names) => Image::Names(names),
        None => Image::Kept,
    }
}

/// The file names of the loose objects a 64-bit Mach-O file's debug map names (its `N_OSO`
/// symbols); objects inside a library (`….rlib(….o)`) are not loose and left out. `None` when
/// the file is not one.
fn debug_map(file: &mut (impl Read + Seek)) -> Option<HashSet<String>> {
    let header = read_at(file, 0, 32)?;
    if u32_at(&header, 0) != MH_MAGIC_64 {
        return None;
    }
    let count = u32_at(&header, 16);
    let commands = read_at(file, 32, u32_at(&header, 20) as usize)?;
    let mut at = 0;
    let mut table = None;
    for _ in 0..count {
        let command = commands.get(at..at + 8)?;
        let size = u32_at(command, 4) as usize;
        if u32_at(command, 0) == LC_SYMTAB {
            let symtab = commands.get(at..at + 24)?;
            table = Some((
                u64::from(u32_at(symtab, 8)),
                u64::from(u32_at(symtab, 12)),
                u64::from(u32_at(symtab, 16)),
            ));
            break;
        }
        // Every command is a whole number of 8-byte words: anything else is not a Mach-O file.
        if size < 8 || !size.is_multiple_of(8) {
            return None;
        }
        at += size;
    }
    let (symbols, count, strings) = table?;
    let entries = read_at(
        file,
        symbols,
        usize::try_from(count).ok()?.checked_mul(NLIST_64)?,
    )?;
    let mut names = HashSet::new();
    for entry in entries.as_chunks::<NLIST_64>().0 {
        if entry[4] != N_OSO {
            continue;
        }
        let text = read_at_most(file, strings + u64::from(u32_at(entry, 0)), LONGEST_PATH)?;
        let path = &text[..text.iter().position(|&byte| byte == 0)?];
        let path = String::from_utf8_lossy(path);
        let name = path.rsplit('/').next().unwrap_or(&path);
        if !name.contains('(') {
            names.insert(name.to_string());
        }
    }
    Some(names)
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Exactly `len` bytes from `at`, if the file has them: a damaged table's size is never allocated.
fn read_at(file: &mut (impl Read + Seek), at: u64, len: usize) -> Option<Vec<u8>> {
    let end = file.seek(SeekFrom::End(0)).ok()?;
    if at.checked_add(len as u64)? > end {
        return None;
    }
    let mut bytes = vec![0; len];
    file.seek(SeekFrom::Start(at)).ok()?;
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

/// Up to `len` bytes from `at`: fewer at the end of the file.
fn read_at_most(file: &mut (impl Read + Seek), at: u64, len: usize) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(at)).ok()?;
    let mut bytes = Vec::with_capacity(len);
    file.take(len as u64).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A 64-bit Mach-O file with one other load command, then a symbol table of the given
    /// `(type, string)` symbols.
    fn mach_o(symbols: &[(u8, &str)]) -> Vec<u8> {
        let mut strings = vec![0u8];
        let mut entries = Vec::new();
        for (kind, text) in symbols {
            entries.extend((strings.len() as u32).to_le_bytes());
            entries.push(*kind);
            entries.extend([0u8; 11]);
            strings.extend(text.as_bytes());
            strings.push(0);
        }
        let symbols_at = 32 + 8 + 16 + 24;
        let strings_at = symbols_at + entries.len();
        let mut file = Vec::new();
        for word in [MH_MAGIC_64, 0x0100_000c, 0, 2, 3, 48, 0, 0] {
            file.extend(word.to_le_bytes());
        }
        // Unrelated commands before the symbol table's: one of the smallest size, one longer.
        for word in [0x2a_u32, 8, 0x1b, 16, 0, 0] {
            file.extend(word.to_le_bytes());
        }
        for word in [
            LC_SYMTAB,
            24,
            symbols_at as u32,
            symbols.len() as u32,
            strings_at as u32,
            strings.len() as u32,
        ] {
            file.extend(word.to_le_bytes());
        }
        file.extend(entries);
        file.extend(strings);
        file
    }

    fn set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn the_build_folder_is_cargos_own_choice_or_the_repositorys() {
        assert_eq!(target_folder(Some("/b".into())), PathBuf::from("/b"));
        assert_eq!(target_folder(None), crate::util::root().join("target"));
    }

    #[test]
    fn every_folder_counts_toward_what_a_sweep_says() {
        let target = tempfile::tempdir().unwrap();
        assert!(sweep_all(target.path(), target.path(), false, true).is_empty());
        let mut objects = Vec::new();
        for (profile, size) in [("debug", 1_500_000), ("release", 2_000_000)] {
            let deps = target.path().join(profile).join("deps");
            std::fs::create_dir_all(&deps).unwrap();
            objects.push(deps.join("gone-1.a.b.rcgu.o"));
            std::fs::write(objects.last().unwrap(), vec![0; size]).unwrap();
            crate::superseded::tests::replaced_build(
                &target.path().join(profile),
                size as u64 * 1000,
            );
        }
        assert_eq!(
            sweep_all(target.path(), target.path(), true, true),
            [
                "Would delete 2 builds no current build uses (3.5 GB).",
                "Would delete 2 object files no program uses (3 MB).",
            ]
        );
        assert!(
            objects.iter().all(|object| object.exists()),
            "a dry run deletes nothing"
        );
        assert_eq!(
            sweep_all(target.path(), target.path(), false, false),
            ["Deleted 2 builds no current build uses (3.5 GB)."],
            "objects only where asked"
        );
        assert_eq!(
            sweep_all(target.path(), target.path(), false, true),
            ["Deleted 2 object files no program uses (3 MB)."]
        );
        assert!(sweep_all(target.path(), target.path(), false, true).is_empty());
    }

    #[test]
    fn a_workspace_loses_the_builds_of_targets_it_no_longer_has() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join("src.rs"), "").unwrap();
        std::fs::write(root.join("tests/kept.rs"), "").unwrap();
        let profile = root.join("target/debug");
        let deps = profile.join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        let json = "{\"features\":\"[]\",\"deps\":[]}";
        for (folder, stem) in [
            ("core-0000000000000001", "test-integration-test-gone"),
            ("core-0000000000000002", "test-integration-test-kept"),
        ] {
            crate::superseded::tests::write_unit(&profile, folder, stem, json, 1, 0);
            let hash = folder.rsplit('-').next().unwrap();
            let test = std::fs::File::create(deps.join(format!("x-{hash}"))).unwrap();
            test.set_len(1_500_000_000).unwrap();
        }
        assert_eq!(
            sweep_all(&root.join("target"), root, true, false),
            ["Would delete 1 build no current build uses (1.5 GB)."]
        );
        assert_eq!(
            sweep_all(&root.join("target"), &root.join("elsewhere"), true, false),
            Vec::<String>::new(),
            "without the workspace's targets, every target is kept"
        );
        sweep_all(&root.join("target"), root, false, false);
        assert!(!deps.join("x-0000000000000001").exists());
        assert!(deps.join("x-0000000000000002").exists());
    }

    #[test]
    fn the_mutants_tree_loses_its_replaced_builds_unless_a_push_holds_it() {
        let checkout = tempfile::tempdir().unwrap();
        assert!(
            idle_mutants_tree(checkout.path(), true).is_empty(),
            "no tree"
        );
        let target = checkout.path().join(".data/mutants-tree/target");
        let deps = target.join("debug").join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        let object = deps.join("gone-1.a.b.rcgu.o");
        std::fs::write(&object, b"o").unwrap();
        crate::superseded::tests::replaced_build(&target.join("debug"), 2_000_000_000);
        let held = crate::mutants_tree::idle_target(checkout.path()).unwrap();
        assert!(
            idle_mutants_tree(checkout.path(), false).is_empty(),
            "a push is testing in it"
        );
        drop(held);
        assert_eq!(
            idle_mutants_tree(checkout.path(), true),
            ["Mutants tree: Would delete 1 build no current build uses (2.0 GB)."]
        );
        assert_eq!(
            idle_mutants_tree(checkout.path(), false),
            ["Mutants tree: Deleted 1 build no current build uses (2.0 GB)."]
        );
        assert!(idle_mutants_tree(checkout.path(), false).is_empty());
        assert!(object.exists(), "only the replaced builds");
    }

    #[test]
    fn a_sweep_says_what_it_deleted_and_nothing_else() {
        assert_eq!(summary(0, 0, false), None);
        assert_eq!(
            summary(1, 2_500_000, false).as_deref(),
            Some("Deleted 1 object file no program uses (2 MB).")
        );
        assert_eq!(
            summary(18_239, 2_602_000_000, true).as_deref(),
            Some("Would delete 18239 object files no program uses (2602 MB).")
        );
        assert_eq!(units_summary(0, 0, true), None);
        assert_eq!(
            units_summary(1, 1_250_000_000, false).as_deref(),
            Some("Deleted 1 build no current build uses (1.2 GB).")
        );
        assert_eq!(
            units_summary(277, 10_960_000_000, true).as_deref(),
            Some("Would delete 277 builds no current build uses (11.0 GB).")
        );
    }

    #[test]
    fn a_debug_map_names_the_loose_objects_only() {
        let file = mach_o(&[
            (0x0f, "_main"),
            (N_OSO, "/t/debug/deps/app-0123.a1.x7.rcgu.o"),
            (
                N_OSO,
                "/t/debug/deps/libcore-4567.rlib(core-4567.b2.y8.rcgu.o)",
            ),
            (0x24, "/t/debug/deps/app-0123.b2.x7.rcgu.o"),
            (N_OSO, "app-0123.c3.x7.rcgu.o"),
        ]);
        assert_eq!(
            debug_map(&mut Cursor::new(file)),
            Some(set(&["app-0123.a1.x7.rcgu.o", "app-0123.c3.x7.rcgu.o"]))
        );
    }

    #[test]
    fn a_file_that_is_not_mach_o_has_no_debug_map() {
        assert_eq!(debug_map(&mut Cursor::new(b"\x7fELF".repeat(16))), None);
        assert_eq!(debug_map(&mut Cursor::new(vec![0xcf, 0xfa])), None);
        let mut cut = mach_o(&[(N_OSO, "/t/a-1.b.c.rcgu.o")]);
        cut.truncate(60);
        assert_eq!(debug_map(&mut Cursor::new(cut)), None);
        // A symbol table bigger than the file.
        let mut huge = mach_o(&[(N_OSO, "/t/a-1.b.c.rcgu.o")]);
        huge[68..72].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(debug_map(&mut Cursor::new(huge)), None);
        // A load command too short to step past.
        let mut looping = mach_o(&[]);
        looping[36..40].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(debug_map(&mut Cursor::new(looping)), None);
        // One that is not a whole number of words.
        let mut ragged = mach_o(&[]);
        ragged[36..40].copy_from_slice(&12u32.to_le_bytes());
        assert_eq!(debug_map(&mut Cursor::new(ragged)), None);
        // No symbol table among the commands.
        let mut without = mach_o(&[(N_OSO, "/t/a-1.b.c.rcgu.o")]);
        without[16..20].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(debug_map(&mut Cursor::new(without)), None);
    }

    #[test]
    fn a_read_may_end_at_the_end_of_the_file_but_not_past_it() {
        let mut file = Cursor::new(b"abcd".to_vec());
        assert_eq!(read_at(&mut file, 0, 4), Some(b"abcd".to_vec()));
        assert_eq!(read_at(&mut file, 1, 3), Some(b"bcd".to_vec()));
        assert_eq!(read_at(&mut file, 1, 4), None);
        assert_eq!(read_at(&mut file, u64::MAX, 2), None);
        assert_eq!(read_at_most(&mut file, 2, 10), Some(b"cd".to_vec()));
    }

    #[test]
    fn a_path_runs_to_its_terminator_even_at_the_end_of_the_file() {
        let mut file = mach_o(&[(N_OSO, "/t/a-1.b.c.rcgu.o")]);
        assert_eq!(
            debug_map(&mut Cursor::new(file.clone())),
            Some(set(&["a-1.b.c.rcgu.o"]))
        );
        file.pop();
        assert_eq!(debug_map(&mut Cursor::new(file)), None);
    }

    #[test]
    fn only_objects_no_program_names_are_stale() {
        let names: Vec<String> = [
            // A test program and the objects of its current and an earlier build.
            "app-0123",
            "app-0123.d",
            "app-0123.a1.new.rcgu.o",
            "app-0123.a1.old.rcgu.o",
            // A library: its objects are in its .rlib.
            "libcore-4567.rlib",
            "core-4567.b2.x.rcgu.o",
            // A proc-macro, whose own debug map names its objects.
            "libderive-89ab.dylib",
            "derive-89ab.c3.new.rcgu.o",
            "derive-89ab.c3.old.rcgu.o",
            // A program this cannot read keeps its objects.
            "odd-cdef",
            "odd-cdef.d4.x.rcgu.o",
            // Not objects.
            "app-0123.rcgu.o",
            "notes.o",
        ]
        .map(String::from)
        .to_vec();
        let asked = std::cell::RefCell::new(Vec::new());
        let stale = stale(&names, |image| {
            asked.borrow_mut().push(image.to_string());
            match image {
                "app-0123" => Image::Names(set(&["app-0123.a1.new.rcgu.o"])),
                "libderive-89ab.dylib" => Image::Names(set(&["derive-89ab.c3.new.rcgu.o"])),
                _ => Image::Kept,
            }
        });
        assert_eq!(
            stale,
            [
                "app-0123.a1.old.rcgu.o",
                "core-4567.b2.x.rcgu.o",
                "derive-89ab.c3.old.rcgu.o"
            ]
        );
        // Each program's file is read once.
        assert_eq!(
            *asked.borrow(),
            ["app-0123", "libderive-89ab.dylib", "odd-cdef"]
        );
    }

    #[test]
    fn only_a_program_built_since_the_last_sweep_is_read_again() {
        let profile = tempfile::tempdir().unwrap();
        let deps = profile.path().join("deps");
        std::fs::create_dir(&deps).unwrap();
        // A program whose debug map names none of its objects.
        let program = deps.join("app-1");
        std::fs::write(&program, mach_o(&[(0x0f, "_main")])).unwrap();
        let object = deps.join("app-1.a.b.rcgu.o");
        let built = |seconds_ago: u64| {
            let time = std::time::SystemTime::now() - std::time::Duration::from_secs(seconds_ago);
            File::options()
                .write(true)
                .open(&program)
                .unwrap()
                .set_modified(time)
                .unwrap();
        };
        std::fs::write(&object, b"o").unwrap();
        assert_eq!(
            sweep(&deps, false),
            (1, 1),
            "the first sweep reads every program"
        );
        assert!(profile.path().join(SWEPT).exists());
        std::fs::write(&object, b"o").unwrap();
        built(60);
        assert_eq!(
            sweep(&deps, false),
            (0, 0),
            "a program built before the last sweep is not read"
        );
        assert!(object.exists());
        built(0);
        std::fs::File::options()
            .write(true)
            .open(profile.path().join(SWEPT))
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(30))
            .unwrap();
        assert_eq!(sweep(&deps, false), (1, 1), "one built since is");
    }

    #[test]
    fn the_deps_folders_of_every_profile_and_target_are_swept() {
        let target = tempfile::tempdir().unwrap();
        for folder in [
            "debug/deps",
            "release/deps",
            "llvm-cov-target/debug/deps",
            "x86_64-pc-windows-msvc/debug/deps",
            "debug/build/x-1/deps/deps",
            "debug/incremental",
        ] {
            std::fs::create_dir_all(target.path().join(folder)).unwrap();
        }
        assert_eq!(
            deps_folders(target.path()),
            [
                "debug/deps",
                "llvm-cov-target/debug/deps",
                "release/deps",
                "x86_64-pc-windows-msvc/debug/deps"
            ]
            .map(|folder| target.path().join(folder))
        );
    }

    #[test]
    fn a_running_build_keeps_its_folder_untouched() {
        let profile = tempfile::tempdir().unwrap();
        let deps = profile.path().join("deps");
        std::fs::create_dir(&deps).unwrap();
        std::fs::write(deps.join("gone-1.a.b.rcgu.o"), b"1234").unwrap();
        let lock = profile.path().join(".cargo-build-lock");
        std::fs::write(&lock, b"").unwrap();
        let cargo = File::options().write(true).open(&lock).unwrap();
        cargo.lock().unwrap();
        crate::superseded::tests::replaced_build(profile.path(), 5);
        assert_eq!(sweep(&deps, false), (0, 0));
        assert_eq!(superseded(&deps, None, false), (0, 0));
        assert!(deps.join("gone-1.a.b.rcgu.o").exists());
        cargo.unlock().unwrap();
        assert_eq!(sweep(&deps, false), (1, 4));
        assert_eq!(superseded(&deps, None, true).0, 1, "a dry run counts");
        assert_eq!(superseded(&deps, None, false).0, 1);
        assert_eq!(superseded(&deps, None, false), (0, 0));
        assert!(!deps.join("gone-1.a.b.rcgu.o").exists());
    }

    /// A real program, built as Cargo builds one into `deps` (`--out-dir`, `extra-filename`) with
    /// its objects kept beside it: those its debug map names stay, a leftover goes.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_built_program_keeps_its_own_objects() {
        let profile = tempfile::tempdir().unwrap();
        let deps = profile.path().join("deps");
        std::fs::create_dir(&deps).unwrap();
        std::fs::write(deps.join("main.rs"), "fn main() {}\n").unwrap();
        let built = std::process::Command::new("rustc")
            .current_dir(&deps)
            .args([
                "main.rs",
                "--crate-name",
                "probe",
                "-C",
                "extra-filename=-0123456789abcdef",
                "-C",
                "debuginfo=line-tables-only",
                "-C",
                "split-debuginfo=unpacked",
                "--out-dir",
                ".",
            ])
            .status()
            .unwrap();
        assert!(built.success());
        let kept: Vec<String> = std::fs::read_dir(&deps)
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with(".rcgu.o"))
            .collect();
        assert!(!kept.is_empty());
        std::fs::write(deps.join("probe-0123456789abcdef.old.x.rcgu.o"), b"12").unwrap();
        assert_eq!(sweep(&deps, false), (1, 2));
        for name in kept {
            assert!(deps.join(name).exists());
        }
    }
}
