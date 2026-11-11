//! Deletes the build folders of the repository's other worktrees that nobody is building in.
//!
//! Every worktree builds into its own `target` (6–37 GB), and a worktree outlives its work: on
//! 2026-11-11 about 45 of them filled a 926 GB disk to 1.2 GB free, and builds failed for want of
//! space. The sweep of a build folder (`sweep.rs`) only ever saw its own checkout's.
//!
//! So every sweep also deletes another worktree's `target`
//! - once its commit is on `origin/main` and it has not been built for [`MERGED_IDLE`], or
//! - once it has not been built for [`IDLE`],
//!
//! and while the disk has less than [`LOW`] free, the least recently built of the rest until it
//! has. It never deletes a build folder while a build holds Cargo's lock on it, nor the mutants
//! tree of a push in progress, nor anything but `target`: the worktree, its changes and its
//! commits stay. A deleted build folder costs its worktree one full build.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::util::Result;

/// How long a worktree whose commit is on `origin/main` keeps its build folder after its last
/// build: long enough for a session that just started from main to build again.
const MERGED_IDLE: Duration = Duration::from_secs(2 * 3600);
/// How long any other worktree keeps its build folder after its last build.
const IDLE: Duration = Duration::from_secs(3 * 24 * 3600);
/// Below this much free space, idle build folders go, least recently built first.
const LOW: u64 = 50_000_000_000;
/// Below this much free space after that, a build is refused: one full build of the workspace
/// writes about 15 GB.
const TOO_LOW: u64 = 15_000_000_000;

/// A worktree of the repository, as `git worktree list --porcelain` lists it.
#[derive(Debug, PartialEq)]
struct Worktree {
    path: PathBuf,
    head: String,
}

/// Another worktree's build folder.
#[derive(Debug)]
struct Folder {
    target: PathBuf,
    /// When a build last wrote to it.
    built: Option<SystemTime>,
    /// Whether its worktree's commit is on `origin/main`.
    merged: bool,
}

/// Why a build folder goes even with space to spare.
#[derive(Debug, PartialEq)]
enum Why {
    Merged(Duration),
    Idle(Duration),
}

/// Deletes the build folders of the worktrees of the repository at `root`, other than `root`'s
/// own, that are merged and idle, or long idle, then while space is low the least recently built
/// of the rest: what to say about it. With `dry_run`, deletes nothing and says what would go.
pub fn sweep(root: &Path, dry_run: bool) -> Vec<String> {
    sweep_at(root, SystemTime::now(), dry_run, free_space)
}

fn sweep_at(
    root: &Path,
    now: SystemTime,
    dry_run: bool,
    free: impl Fn(&Path) -> Option<u64>,
) -> Vec<String> {
    let worktrees = match crate::util::git(root, &["worktree", "list", "--porcelain"], &[0]) {
        Ok(text) => parse(&text),
        Err(cause) => return vec![format!("Could not list the other worktrees: {cause}")],
    };
    let own = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut folders: Vec<Folder> = worktrees
        .iter()
        .filter(|worktree| std::fs::canonicalize(&worktree.path).ok().as_ref() != Some(&own))
        .filter_map(|worktree| folder(root, worktree))
        .collect();
    let mut messages = Vec::new();
    let verb = if dry_run { "Would delete" } else { "Deleted" };
    folders.retain(|folder| match why(folder, now) {
        Some(why) => {
            if dry_run || delete(&folder.target) {
                messages.push(format!(
                    "{verb} {}: {}.",
                    folder.target.display(),
                    describe(&why)
                ));
            }
            false
        }
        None => true,
    });
    if free(root).is_some_and(|bytes| bytes < LOW) {
        for folder in least_recent_first(folders) {
            if free(root).is_none_or(|bytes| bytes >= LOW) {
                break;
            }
            if dry_run || delete(&folder.target) {
                messages.push(format!(
                    "{verb} {}: the disk has less than {} GB free.",
                    folder.target.display(),
                    LOW / 1_000_000_000
                ));
            }
            if dry_run {
                break;
            }
        }
    }
    messages
}

/// Refuses to build while the disk at `root` has less than [`TOO_LOW`] free, after the sweep:
/// a build that runs out of space fails halfway, in whatever step it was. Where the free space
/// cannot be read, says nothing against it.
pub fn enough_space(root: &Path) -> Result {
    match free_space(root) {
        Some(bytes) if bytes < TOO_LOW => Err(format!(
            "Only {:.1} GB is free on this disk, and a build needs about {} GB. Free some space \
             (remove worktrees you are done with: `git worktree remove <path>`) and try again.",
            bytes as f64 / 1e9,
            TOO_LOW / 1_000_000_000
        )),
        _ => Ok(()),
    }
}

/// The worktrees in `git worktree list --porcelain` output: those with a checkout, not the bare
/// repository or ones whose folder is gone.
fn parse(porcelain: &str) -> Vec<Worktree> {
    porcelain
        .split("\n\n")
        .filter_map(|block| {
            let mut path = None;
            let mut head = None;
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("worktree ") {
                    path = Some(PathBuf::from(rest));
                } else if let Some(rest) = line.strip_prefix("HEAD ") {
                    head = Some(rest.to_string());
                } else if line == "bare" || line.starts_with("prunable") {
                    return None;
                }
            }
            Some(Worktree {
                path: path?,
                head: head?,
            })
        })
        .collect()
}

/// `worktree`'s build folder, when it has one.
fn folder(root: &Path, worktree: &Worktree) -> Option<Folder> {
    let target = worktree.path.join("target");
    if !target.is_dir() {
        return None;
    }
    Some(Folder {
        built: last_built(&target),
        merged: crate::util::git(
            root,
            &["merge-base", "--is-ancestor", &worktree.head, "origin/main"],
            &[0],
        )
        .is_ok(),
        target,
    })
}

/// When a build last wrote to `target`: the newest of the folders a build adds files to.
fn last_built(target: &Path) -> Option<SystemTime> {
    let mut folders = vec![target.to_path_buf()];
    for deps in crate::sweep::deps_folders(target) {
        if let Some(profile) = deps.parent() {
            folders.push(profile.join(".fingerprint"));
            folders.push(profile.join("incremental"));
        }
        folders.push(deps);
    }
    folders
        .iter()
        .filter_map(|folder| crate::util::modified(folder))
        .max()
}

/// Why `folder` goes at `now` whatever the free space: merged and idle, or long idle. A folder
/// whose last build cannot be told stays.
fn why(folder: &Folder, now: SystemTime) -> Option<Why> {
    let idle = now.duration_since(folder.built?).unwrap_or_default();
    if folder.merged && idle >= MERGED_IDLE {
        Some(Why::Merged(idle))
    } else if idle >= IDLE {
        Some(Why::Idle(idle))
    } else {
        None
    }
}

fn describe(why: &Why) -> String {
    match why {
        Why::Merged(idle) => format!("its work is on main, not built for {}", ago(*idle)),
        Why::Idle(idle) => format!("not built for {}", ago(*idle)),
    }
}

/// `duration` in hours under two days, else in days.
fn ago(duration: Duration) -> String {
    let hours = duration.as_secs() / 3600;
    if hours < 48 {
        format!("{hours} h")
    } else {
        format!("{} days", hours / 24)
    }
}

/// `folders`, the least recently built first; those whose last build cannot be told stay out.
fn least_recent_first(folders: Vec<Folder>) -> Vec<Folder> {
    let mut known: Vec<Folder> = folders
        .into_iter()
        .filter(|folder| folder.built.is_some())
        .collect();
    known.sort_by_key(|folder| folder.built);
    known
}

/// Deletes the build folder `target` unless a build holds Cargo's lock on one of its profile
/// folders (or the mutants tree's lock, for a mutants tree's): whether it went.
///
/// Its contents go while the locks are held, so no build starts in it meanwhile; the lock files
/// themselves go after, since Windows deletes no file that is open. A build that starts in
/// between finds an empty folder and builds it again.
fn delete(target: &Path) -> bool {
    let Some(_tree) = mutants_tree_lock(target) else {
        return false;
    };
    let mut locks = Vec::new();
    for deps in crate::sweep::deps_folders(target) {
        match deps.parent().and_then(crate::sweep::build_lock) {
            Some(held) => locks.extend(held),
            None => return false,
        }
    }
    let held: Vec<PathBuf> = crate::sweep::deps_folders(target)
        .iter()
        .filter_map(|deps| deps.parent())
        .flat_map(|profile| crate::sweep::LOCKS.map(|name| profile.join(name)))
        .collect();
    remove_all_but(target, &held);
    drop(locks);
    std::fs::remove_dir_all(target).is_ok()
}

/// For the build folder of a checkout's mutants tree (`<checkout>/.data/mutants-tree/target`),
/// the tree's lock, held, or `None` while a push uses the tree; for any other folder, a lock
/// that holds nothing.
fn mutants_tree_lock(target: &Path) -> Option<Option<File>> {
    let checkout = target
        .parent()
        .filter(|tree| tree.ends_with(crate::mutants_tree::TREE))
        .and_then(|tree| tree.parent())
        .and_then(Path::parent);
    match checkout {
        Some(checkout) => crate::mutants_tree::idle_target(checkout).map(|(_, lock)| Some(lock)),
        None => Some(None),
    }
}

/// Deletes everything inside `folder` except the files in `kept` and the folders that hold them.
fn remove_all_but(folder: &Path, kept: &[PathBuf]) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if kept.contains(&path) {
            continue;
        }
        if kept.iter().any(|file| file.starts_with(&path)) {
            remove_all_but(&path, kept);
        } else if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// The bytes free for this user on the disk holding `path`, when the system says: `df` on Unix,
/// .NET's `DriveInfo` through PowerShell on Windows.
fn free_space(path: &Path) -> Option<u64> {
    let mut command = if cfg!(windows) {
        let mut command = std::process::Command::new("powershell");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "[System.IO.DriveInfo]::new('{}').AvailableFreeSpace",
                path.display().to_string().replace('\'', "''")
            ),
        ]);
        command
    } else {
        let mut command = std::process::Command::new("df");
        command.arg("-Pk").arg(path);
        command
    };
    let out = command.output().ok().filter(|out| out.status.success())?;
    let text = String::from_utf8_lossy(&out.stdout);
    if cfg!(windows) {
        text.trim().parse().ok()
    } else {
        df_available(&text)
    }
}

/// The bytes available in `df -Pk` output: its second line's fourth column, in kilobytes.
fn df_available(text: &str) -> Option<u64> {
    let kilobytes: u64 = text
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()?;
    kilobytes.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::git;

    const HOUR: Duration = Duration::from_secs(3600);

    fn folder(built: Option<SystemTime>, merged: bool) -> Folder {
        Folder {
            target: PathBuf::from("t"),
            built,
            merged,
        }
    }

    #[test]
    fn a_merged_folder_goes_after_two_hours_and_any_other_after_three_days() {
        let now = SystemTime::UNIX_EPOCH + 1000 * HOUR;
        let built = |hours: u32| Some(now - HOUR * hours);
        assert_eq!(why(&folder(built(1), true), now), None);
        assert_eq!(
            why(&folder(built(2), true), now),
            Some(Why::Merged(2 * HOUR))
        );
        assert_eq!(why(&folder(built(71), false), now), None);
        assert_eq!(
            why(&folder(built(72), false), now),
            Some(Why::Idle(72 * HOUR))
        );
        assert_eq!(why(&folder(None, true), now), None, "unknown age stays");
        assert_eq!(
            why(&folder(Some(now + HOUR), true), now),
            None,
            "built after now"
        );
    }

    #[test]
    fn durations_read_in_hours_then_days() {
        assert_eq!(ago(47 * HOUR), "47 h");
        assert_eq!(ago(48 * HOUR), "2 days");
        assert_eq!(
            describe(&Why::Merged(3 * HOUR)),
            "its work is on main, not built for 3 h"
        );
        assert_eq!(describe(&Why::Idle(96 * HOUR)), "not built for 4 days");
    }

    #[test]
    fn worktrees_are_read_from_git_s_porcelain_list() {
        let text = "worktree /r\nHEAD aaa\nbranch refs/heads/main\n\n\
                    worktree /w\nHEAD bbb\ndetached\nlocked reason\n\n\
                    worktree /bare\nbare\n\n\
                    worktree /gone\nHEAD ccc\nbranch refs/heads/x\nprunable gitdir file points to non-existent location\n";
        assert_eq!(
            parse(text),
            [
                Worktree {
                    path: "/r".into(),
                    head: "aaa".into()
                },
                Worktree {
                    path: "/w".into(),
                    head: "bbb".into()
                },
            ]
        );
    }

    #[test]
    fn the_least_recently_built_go_first_and_unknown_ages_never() {
        let at = |hours: u32| Some(SystemTime::UNIX_EPOCH + HOUR * hours);
        let order: Vec<_> = least_recent_first(vec![
            folder(at(5), false),
            folder(None, false),
            folder(at(1), true),
            folder(at(3), false),
        ])
        .iter()
        .map(|folder| folder.built)
        .collect();
        assert_eq!(order, [at(1), at(3), at(5)]);
    }

    #[test]
    fn free_space_is_df_s_available_kilobytes() {
        let text = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                    /dev/disk3s5 971350180 530000000 398000000 58% /System/Volumes/Data\n";
        assert_eq!(df_available(text), Some(398_000_000 * 1024));
        assert_eq!(df_available("Filesystem\n"), None);
        assert_eq!(df_available("header\n/dev/x 1 2 lots 3% /\n"), None);
    }

    #[test]
    fn this_disk_s_free_space_can_be_read() {
        assert!(free_space(&crate::util::root()).is_some_and(|bytes| bytes > 0));
    }

    /// A repository with one commit on `origin/main`, and a worktree beside it at `head`.
    fn repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let run = |at: &Path, args: &[&str]| git(at, args, &[0]).unwrap().trim().to_string();
        let commit = |at: &Path, message: &str| {
            run(
                at,
                &[
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-q",
                    "--allow-empty",
                    "-m",
                    message,
                ],
            );
        };
        run(&root, &["init", "-q"]);
        commit(&root, "first");
        run(&root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        let other = dir.path().join("other");
        run(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "work",
                other.to_str().unwrap(),
            ],
        );
        (dir, root, other)
    }

    /// A build folder in `checkout` with one profile and one built file, as Cargo leaves it.
    fn built(checkout: &Path) -> PathBuf {
        let target = checkout.join("target");
        let profile = target.join("debug");
        std::fs::create_dir_all(profile.join("deps")).unwrap();
        std::fs::create_dir_all(profile.join(".fingerprint")).unwrap();
        std::fs::write(profile.join("deps/kchess-0123"), b"program").unwrap();
        std::fs::write(profile.join(".cargo-lock"), b"").unwrap();
        target
    }

    fn sweep_after(root: &Path, idle: Duration, free: u64) -> Vec<String> {
        sweep_at(root, SystemTime::now() + idle, false, |_| Some(free))
    }

    #[test]
    fn a_merged_worktree_s_build_folder_goes_and_the_own_one_stays() {
        let (_dir, root, other) = repo();
        let own = built(&root);
        let theirs = built(&other);
        assert!(
            sweep_after(&root, HOUR, LOW).is_empty(),
            "built an hour ago"
        );
        assert!(theirs.is_dir());
        let said = sweep_after(&root, 3 * HOUR, LOW);
        assert!(!theirs.exists(), "{said:?}");
        assert!(
            own.is_dir(),
            "the checkout's own build folder is its own sweep's"
        );
        assert!(other.join(".git").exists(), "only the build folder goes");
        assert_eq!(said.len(), 1);
        assert!(said[0].ends_with("its work is on main, not built for 3 h."));
    }

    #[test]
    fn unmerged_work_keeps_its_build_folder_for_three_days() {
        let (_dir, root, other) = repo();
        git(
            &other,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "unmerged",
            ],
            &[0],
        )
        .unwrap();
        let theirs = built(&other);
        assert!(sweep_after(&root, 71 * HOUR, LOW).is_empty());
        assert!(theirs.is_dir());
        let said = sweep_after(&root, 73 * HOUR, LOW);
        assert!(!theirs.exists());
        assert!(said[0].ends_with("not built for 3 days."), "{said:?}");
    }

    #[test]
    fn low_space_takes_recent_folders_too_but_never_one_being_built() {
        let (_dir, root, other) = repo();
        git(
            &other,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "unmerged",
            ],
            &[0],
        )
        .unwrap();
        let theirs = built(&other);
        let lock = File::options()
            .write(true)
            .open(theirs.join("debug/.cargo-lock"))
            .unwrap();
        lock.try_lock().unwrap();
        assert!(sweep_after(&root, Duration::ZERO, LOW - 1).is_empty());
        assert!(theirs.is_dir(), "a build holds its lock");
        drop(lock);
        let said = sweep_after(&root, Duration::ZERO, LOW - 1);
        assert!(!theirs.exists());
        assert!(
            said[0].ends_with("the disk has less than 50 GB free."),
            "{said:?}"
        );
    }

    #[test]
    fn with_space_to_spare_recent_unmerged_work_stays() {
        let (_dir, root, other) = repo();
        let theirs = built(&other);
        std::fs::write(other.join("change"), b"").unwrap();
        git(&other, &["add", "change"], &[0]).unwrap();
        git(
            &other,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "change",
            ],
            &[0],
        )
        .unwrap();
        assert!(sweep_after(&root, HOUR, LOW).is_empty());
        assert!(theirs.is_dir());
    }

    #[test]
    fn a_dry_run_deletes_nothing() {
        let (_dir, root, other) = repo();
        let theirs = built(&other);
        let said = sweep_at(&root, SystemTime::now() + 3 * HOUR, true, |_| Some(LOW));
        assert!(theirs.is_dir());
        assert!(said[0].starts_with("Would delete"), "{said:?}");
    }

    #[test]
    fn everything_but_the_kept_files_goes() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("debug");
        std::fs::create_dir_all(profile.join("deps")).unwrap();
        std::fs::write(profile.join("deps/a"), b"").unwrap();
        std::fs::write(profile.join("other"), b"").unwrap();
        std::fs::write(profile.join(".cargo-lock"), b"").unwrap();
        std::fs::create_dir_all(dir.path().join("release")).unwrap();
        remove_all_but(dir.path(), &[profile.join(".cargo-lock")]);
        let left: Vec<_> = walk(dir.path());
        assert_eq!(left, [profile.clone(), profile.join(".cargo-lock")]);
    }

    fn walk(folder: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for path in std::fs::read_dir(folder)
            .unwrap()
            .flatten()
            .map(|e| e.path())
        {
            found.push(path.clone());
            if path.is_dir() {
                found.extend(walk(&path));
            }
        }
        found.sort();
        found
    }
}
