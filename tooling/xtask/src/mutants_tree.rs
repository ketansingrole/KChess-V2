//! The pre-push hook's own checkout for mutation testing (`check::branch_mutants`): a detached
//! worktree that the hook sets to the pushed commit and mutates in place, so each push rebuilds
//! only what changed since the last one, and never touches the files someone is editing.
//!
//! Each checkout keeps its own, in its (ignored) `.data`: two sessions in two worktrees never
//! share one, and it goes when its checkout is removed. A lock beside it makes a second push
//! from the same checkout wait. Whatever an interrupted run left (a mutated file, a half-made
//! tree) is put right before the next run starts.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};

use crate::util::{Result, git, say};

/// The tree, from the checkout.
const TREE: &str = ".data/mutants-tree";

/// The lock held while the tree is in use, from the checkout.
const LOCK: &str = ".data/mutants-tree.lock";

/// A checkout's mutants tree, set to one commit and locked until this is dropped.
pub struct Tree {
    pub path: PathBuf,
    _lock: File,
}

impl Tree {
    /// Its build folder.
    pub fn target(&self) -> PathBuf {
        self.path.join("target")
    }
}

/// `checkout`'s tree's build folder, with the tree's lock held until the file is dropped: none
/// when it has none, or while a push is using the tree.
pub fn idle_target(checkout: &Path) -> Option<(PathBuf, File)> {
    let target = checkout.join(TREE).join("target");
    if !target.is_dir() {
        return None;
    }
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(checkout.join(LOCK))
        .ok()?;
    lock.try_lock().ok()?;
    Some((target, lock))
}

/// `checkout`'s tree, locked (after any other run from `checkout` ends) and holding exactly
/// `commit`: its files, and nothing else but its build output.
pub fn prepare(checkout: &Path, commit: &str) -> Result<Tree> {
    let lock = lock(&checkout.join(LOCK))?;
    let path = checkout.join(TREE);
    sync(checkout, &path, commit)?;
    Ok(Tree { path, _lock: lock })
}

/// The lock file at `path`, locked: at once, or once its holder lets go. The system lets go of a
/// lock when its process ends, however it ends, so an interrupted run never leaves it held.
fn lock(path: &Path) -> Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|cause| format!("{}: {cause}", dir.display()))?;
    }
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|cause| format!("{}: {cause}", path.display()))?;
    match file.try_lock() {
        Ok(()) => return Ok(file),
        Err(TryLockError::WouldBlock) => {
            say("Another push from this checkout is testing its mutants: waiting for it…");
        }
        Err(TryLockError::Error(cause)) => return Err(format!("{}: {cause}", path.display())),
    }
    file.lock()
        .map_err(|cause| format!("{}: {cause}", path.display()))?;
    Ok(file)
}

/// Sets `tree` to exactly `commit`, making it `checkout`'s worktree first when it is not one.
/// An existing tree keeps its build output (`/target`) and the unchanged files' times, so the
/// next build is incremental; anything else is reset. A tree that cannot be reset is made anew.
fn sync(checkout: &Path, tree: &Path, commit: &str) -> Result {
    let wanted = git(
        checkout,
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
        &[0],
    )?;
    let wanted = wanted.trim();
    if is_tree_of(checkout, tree) && reset(tree, wanted).is_ok() && holds(tree, wanted)? {
        return Ok(());
    }
    if tree.exists() {
        say("Making the mutants tree anew: its next build is a full one.");
        std::fs::remove_dir_all(tree).map_err(|cause| format!("{}: {cause}", tree.display()))?;
    }
    git(checkout, &["worktree", "prune"], &[0])?;
    let path = tree.to_string_lossy();
    git(
        checkout,
        &["worktree", "add", "-q", "--detach", &path, wanted],
        &[0],
    )?;
    if holds(tree, wanted)? {
        Ok(())
    } else {
        Err(format!("{}: could not set it to {wanted}.", tree.display()))
    }
}

/// Puts `tree` at `commit`, its tracked files as committed and no other file but `/target`'s.
fn reset(tree: &Path, commit: &str) -> Result {
    git(
        tree,
        &["checkout", "-q", "--force", "--detach", commit],
        &[0],
    )?;
    git(tree, &["clean", "-q", "-ffdx", "-e", "/target"], &[0])?;
    Ok(())
}

/// Whether `tree` is at `commit` with nothing changed or added.
fn holds(tree: &Path, commit: &str) -> Result<bool> {
    let head = git(tree, &["rev-parse", "HEAD"], &[0])?;
    let status = git(
        tree,
        &["status", "--porcelain", "--untracked-files=all"],
        &[0],
    )?;
    Ok(head.trim() == commit && status.trim().is_empty())
}

/// Whether `tree` is a worktree of `checkout`'s repository in its own right. Inside the checkout,
/// a folder without its own `.git` would answer Git as the checkout itself; one with a `.git` file
/// is its own top, or an error when that file is broken.
fn is_tree_of(checkout: &Path, tree: &Path) -> bool {
    if !tree.join(".git").is_file() {
        return false;
    }
    let common = |repo: &Path| {
        git(
            repo,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            &[0],
        )
        .ok()
        .and_then(|dir| std::fs::canonicalize(dir.trim()).ok())
    };
    matches!((common(tree), common(checkout)), (Some(theirs), Some(ours)) if theirs == ours)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository with two commits, and their hashes.
    fn repo() -> (tempfile::TempDir, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let run = |args: &[&str]| git(path, args, &[0]).unwrap().trim().to_string();
        let commit = |message: &str| {
            run(&["add", "-A"]);
            run(&[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                message,
            ]);
            run(&["rev-parse", "HEAD"])
        };
        run(&["init", "-q"]);
        // Windows runners convert line endings on checkout (`core.autocrlf`); the files here
        // must come back byte for byte, whatever the machine's own Git settings.
        run(&["config", "core.autocrlf", "false"]);
        std::fs::write(path.join(".gitignore"), "target/\n.data/\n").unwrap();
        std::fs::write(path.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(path.join("b.rs"), "fn b() {}\n").unwrap();
        let first = commit("first");
        std::fs::write(path.join("a.rs"), "fn a() { 1; }\n").unwrap();
        let second = commit("second");
        (dir, first, second)
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn its_build_folder_is_idle_only_while_no_push_holds_the_tree() {
        let (dir, first, _) = repo();
        let checkout = dir.path();
        let tree = prepare(checkout, &first).unwrap();
        std::fs::create_dir_all(tree.target()).unwrap();
        assert_eq!(tree.target(), checkout.join(".data/mutants-tree/target"));
        assert!(idle_target(checkout).is_none(), "the push holds it");
        drop(tree);
        let (target, _lock) = idle_target(checkout).unwrap();
        assert_eq!(target, checkout.join(".data/mutants-tree/target"));
        std::fs::remove_dir(&target).unwrap();
        assert!(idle_target(checkout).is_none(), "no build folder");
    }

    #[test]
    fn the_tree_is_the_checkout_s_own_and_holds_the_commit() {
        let (dir, first, second) = repo();
        let checkout = dir.path();
        let tree = prepare(checkout, &first).unwrap();
        assert_eq!(tree.path, checkout.join(".data/mutants-tree"));
        assert_eq!(read(&tree.path.join("a.rs")), "fn a() {}\n");
        assert!(is_tree_of(checkout, &tree.path));
        // The checkout itself is untouched: still at its own commit, nothing changed.
        assert!(holds(checkout, &second).unwrap());
        drop(tree);

        // Another worktree of the same repository has a tree of its own.
        let other = tempfile::tempdir().unwrap();
        let other = other.path().join("other");
        let path = other.to_string_lossy();
        git(
            checkout,
            &["worktree", "add", "-q", "--detach", &path, &second],
            &[0],
        )
        .unwrap();
        let theirs = prepare(&other, &second).unwrap();
        assert_eq!(theirs.path, other.join(".data/mutants-tree"));
        assert_eq!(read(&theirs.path.join("a.rs")), "fn a() { 1; }\n");
    }

    #[test]
    fn a_new_commit_changes_only_its_files_and_keeps_the_build() {
        let (dir, first, second) = repo();
        let checkout = dir.path();
        let path = prepare(checkout, &first).unwrap().path;
        std::fs::create_dir_all(path.join("target")).unwrap();
        std::fs::write(path.join("target/built"), "warm").unwrap();
        let untouched = std::fs::metadata(path.join("b.rs"))
            .unwrap()
            .modified()
            .unwrap();

        let tree = prepare(checkout, &second).unwrap();
        assert_eq!(read(&tree.path.join("a.rs")), "fn a() { 1; }\n");
        assert_eq!(read(&tree.path.join("target/built")), "warm");
        let after = std::fs::metadata(tree.path.join("b.rs"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(after, untouched, "an unchanged file was rewritten");
    }

    #[test]
    fn what_an_interrupted_run_left_is_put_right() {
        let (dir, _, second) = repo();
        let checkout = dir.path();
        let path = prepare(checkout, &second).unwrap().path;
        // A mutant left in place, a stray file, and a stray ignored file outside the build.
        std::fs::write(path.join("a.rs"), "fn a() { 0; }\n").unwrap();
        std::fs::write(path.join("stray.rs"), "fn stray() {}\n").unwrap();
        std::fs::create_dir_all(path.join(".data")).unwrap();
        std::fs::write(path.join(".data/left"), "x").unwrap();
        git(&path, &["add", "stray.rs"], &[0]).unwrap();

        let tree = prepare(checkout, &second).unwrap();
        assert_eq!(read(&tree.path.join("a.rs")), "fn a() { 1; }\n");
        assert!(!tree.path.join("stray.rs").exists());
        assert!(!tree.path.join(".data").exists());
        assert!(holds(&tree.path, &second).unwrap());
    }

    #[test]
    fn a_tree_that_is_not_a_worktree_is_made_anew() {
        let (dir, _, second) = repo();
        let checkout = dir.path();
        // A folder Git would answer for as the checkout itself.
        let path = checkout.join(TREE);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("junk"), "x").unwrap();
        assert!(!is_tree_of(checkout, &path));
        let tree = prepare(checkout, &second).unwrap();
        assert!(is_tree_of(checkout, &tree.path));
        assert!(!tree.path.join("junk").exists());
        drop(tree);

        // Its worktree's record gone (as `git worktree remove` elsewhere would leave it).
        std::fs::remove_file(path.join(".git")).unwrap();
        let tree = prepare(checkout, &second).unwrap();
        assert!(is_tree_of(checkout, &tree.path));
        assert!(holds(&tree.path, &second).unwrap());
        drop(tree);

        assert!(prepare(checkout, "no-such-commit").is_err());
    }

    #[test]
    fn a_second_run_from_the_checkout_waits_for_the_first() {
        let (dir, _, second) = repo();
        let checkout = dir.path().to_path_buf();
        let first = prepare(&checkout, &second).unwrap();
        let (sent, got) = std::sync::mpsc::channel();
        let waiting = std::thread::spawn({
            let checkout = checkout.clone();
            let second = second.clone();
            move || {
                let tree = prepare(&checkout, &second).unwrap();
                sent.send(()).unwrap();
                drop(tree);
            }
        });
        let held = std::time::Duration::from_millis(300);
        assert!(got.recv_timeout(held).is_err(), "it did not wait");
        drop(first);
        got.recv_timeout(std::time::Duration::from_secs(30))
            .expect("it never ran once the first let go");
        waiting.join().unwrap();
    }

    #[test]
    fn a_tree_holds_a_commit_only_at_it_and_unchanged() {
        let (dir, first, second) = repo();
        let checkout = dir.path();
        assert!(holds(checkout, &second).unwrap());
        assert!(!holds(checkout, &first).unwrap());
        std::fs::write(checkout.join("a.rs"), "fn a() { 2; }\n").unwrap();
        assert!(!holds(checkout, &second).unwrap());
        git(checkout, &["checkout", "-q", "--", "a.rs"], &[0]).unwrap();
        std::fs::write(checkout.join("new.rs"), "fn new() {}\n").unwrap();
        assert!(!holds(checkout, &second).unwrap());
    }

    #[test]
    fn a_worktree_of_another_repository_is_not_the_tree() {
        let (dir, _, second) = repo();
        let checkout = dir.path();
        let (elsewhere, _, theirs) = repo();
        let path = checkout.join(TREE);
        let at = path.to_string_lossy();
        git(
            elsewhere.path(),
            &["worktree", "add", "-q", "--detach", &at, &theirs],
            &[0],
        )
        .unwrap();
        assert!(!is_tree_of(checkout, &path));
        let tree = prepare(checkout, &second).unwrap();
        assert!(is_tree_of(checkout, &tree.path));
        assert!(holds(&tree.path, &second).unwrap());
    }
}
