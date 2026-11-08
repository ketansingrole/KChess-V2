//! A release's program files in memory, as the release tooling compares them.

use std::collections::BTreeMap;
use std::path::Path;

use crate::{Error, Result};

/// A file, link or folder of a [`Tree`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    File { data: Vec<u8>, executable: bool },
    Link(String),
    Dir,
}

/// Program files by `/`-separated path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree(BTreeMap<String, Node>);

impl Tree {
    pub fn new() -> Tree {
        Tree::default()
    }

    pub fn insert(&mut self, path: impl Into<String>, node: Node) {
        self.0.insert(path.into(), node);
    }

    pub fn get(&self, path: &str) -> Option<&Node> {
        self.0.get(path)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Node)> {
        self.0.iter().map(|(path, node)| (path.as_str(), node))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Everything under `root`, but what `skip` names (by path).
    pub fn read(root: &Path, skip: impl Fn(&str) -> bool) -> Result<Tree> {
        let mut tree = Tree::new();
        read_into(&mut tree, root, "", &skip)?;
        Ok(tree)
    }

    /// The part of the tree under `prefix` (`KChess.app`), with the prefix taken off its paths.
    pub fn under(&self, prefix: &str) -> Tree {
        let start = format!("{prefix}/");
        Tree(
            self.0
                .iter()
                .filter_map(|(path, node)| {
                    path.strip_prefix(&start)
                        .filter(|rest| !rest.is_empty())
                        .map(|rest| (rest.to_string(), node.clone()))
                })
                .collect(),
        )
    }
}

fn read_into(
    tree: &mut Tree,
    dir: &Path,
    prefix: &str,
    skip: &impl Fn(&str) -> bool,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| Error(format!("{} is not a UTF-8 name", name.to_string_lossy())))?;
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if skip(&path) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            let target = std::fs::read_link(entry.path())?;
            tree.insert(
                path,
                Node::Link(target.to_string_lossy().replace('\\', "/")),
            );
        } else if kind.is_dir() {
            read_into(tree, &entry.path(), &path, skip)?;
            if !tree
                .0
                .keys()
                .any(|key| key.starts_with(&format!("{path}/")))
            {
                tree.insert(path, Node::Dir);
            }
        } else {
            let data = std::fs::read(entry.path())?;
            let executable = is_executable(&entry.metadata()?);
            tree.insert(path, Node::File { data, executable });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    false
}
