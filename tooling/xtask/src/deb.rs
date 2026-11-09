//! The Debian package format, both ways: the Linux packaging writes a `.deb` (`package/linux.rs`)
//! and the update patches read a release's (`deltas`). A `.deb` is an `ar` archive of
//! `debian-binary`, the control archive and the data archive, each a tar with root ownership.
//!
//! Compiled on every platform, so the tests read back what the writer writes everywhere; only
//! Linux packages with the writer.

use std::io::{Read as _, Write as _};

use kchess_delta::{Node, Tree};

use crate::util::Result;

/// A tar of `entries` (path in the archive → contents and mode), after the folders `dirs`, all
/// owned by root, as dpkg installs them.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn tarball(entries: &[(String, Vec<u8>, u32)], dirs: &[String]) -> Result<Vec<u8>> {
    let mut archive = tar::Builder::new(Vec::new());
    let header = |path: &str, size: u64, mode: u32, kind: tar::EntryType| -> Result<tar::Header> {
        let mut header = tar::Header::new_gnu();
        header.set_path(path).map_err(|cause| cause.to_string())?;
        header.set_size(size);
        header.set_mode(mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(kind);
        header.set_cksum();
        Ok(header)
    };
    for dir in dirs {
        let header = header(&format!("./{dir}/"), 0, 0o755, tar::EntryType::Directory)?;
        archive
            .append(&header, std::io::empty())
            .map_err(|cause| cause.to_string())?;
    }
    for (path, data, mode) in entries {
        let header = header(
            &format!("./{path}"),
            data.len() as u64,
            *mode,
            tar::EntryType::Regular,
        )?;
        archive
            .append(&header, data.as_slice())
            .map_err(|cause| cause.to_string())?;
    }
    archive.into_inner().map_err(|cause| cause.to_string())
}

/// `data`, gzipped (the control archive: a few hundred bytes).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(data).map_err(|cause| cause.to_string())?;
    encoder.finish().map_err(|cause| cause.to_string())
}

/// `data` as xz (the program files: a quarter smaller than gzip, which dpkg has read since
/// 1.15.6). Blocks of 64 MiB are compressed on every core, as `xz -9 -T0` does.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn xz(data: &[u8]) -> Result<Vec<u8>> {
    let mut options = lzma_rust2::XzOptions::with_preset(9);
    options.block_size = std::num::NonZeroU64::new(64 << 20);
    let workers = std::thread::available_parallelism().map_or(1, |cores| cores.get() as u32);
    let mut writer = lzma_rust2::XzWriterMt::new(Vec::new(), options, workers)
        .map_err(|cause| cause.to_string())?;
    writer.write_all(data).map_err(|cause| cause.to_string())?;
    writer.finish().map_err(|cause| cause.to_string())
}

/// The `ar` header's length: name, date, owner, group, mode, size and the closing quote.
const AR_HEADER: usize = 60;

/// An `ar` archive of `members`, as dpkg reads it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn ar(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = b"!<arch>\n".to_vec();
    for (name, data) in members {
        let header = format!(
            "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            name,
            0,
            0,
            0,
            "100644",
            data.len()
        );
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

/// The members of an `ar` archive, by name.
fn ar_members(bytes: &[u8]) -> Result<Vec<(String, &[u8])>> {
    let mut rest = bytes
        .strip_prefix(b"!<arch>\n")
        .ok_or("The .deb is not an ar archive.")?;
    let mut members = Vec::new();
    while rest.len() >= AR_HEADER {
        let (header, body) = rest.split_at(AR_HEADER);
        let name = String::from_utf8_lossy(&header[..16])
            .trim()
            .trim_end_matches('/')
            .to_string();
        let size: usize = String::from_utf8_lossy(&header[48..58])
            .trim()
            .parse()
            .map_err(|_| "The .deb has a member of no size.")?;
        let data = body.get(..size).ok_or("The .deb is cut short.")?;
        members.push((name, data));
        rest = body.get(size + size % 2..).unwrap_or_default();
    }
    Ok(members)
}

/// The files a `.deb` installs (paths from `/`), and its control archive by name: what the
/// updater needs to make a `.deb` of the rebuilt files.
pub fn read(bytes: &[u8]) -> Result<(Tree, (String, Vec<u8>))> {
    let members = ar_members(bytes)?;
    let (control, control_data) = members
        .iter()
        .find(|(name, _)| name.starts_with("control.tar"))
        .ok_or("The .deb has no control archive.")?;
    let (data_name, data) = members
        .iter()
        .find(|(name, _)| name.starts_with("data.tar"))
        .ok_or("The .deb has no data archive.")?;
    let mut tar = Vec::new();
    match data_name.as_str() {
        "data.tar.gz" => flate2::read::GzDecoder::new(*data).read_to_end(&mut tar),
        "data.tar.xz" => lzma_rust2::XzReader::new(*data, true).read_to_end(&mut tar),
        "data.tar" => {
            tar.extend_from_slice(data);
            Ok(0)
        }
        other => return Err(format!("The .deb's {other} is not one this reads.")),
    }
    .map_err(|cause| format!("The .deb's {data_name} could not be read: {cause}"))?;
    let mut tree = Tree::new();
    let mut archive = tar::Archive::new(tar.as_slice());
    for entry in archive.entries().map_err(|cause| cause.to_string())? {
        let mut entry = entry.map_err(|cause| cause.to_string())?;
        let path = entry
            .path()
            .map_err(|cause| cause.to_string())?
            .to_string_lossy()
            .trim_start_matches("./")
            .trim_end_matches('/')
            .to_string();
        if path.is_empty() {
            continue;
        }
        let header = entry.header().clone();
        match header.entry_type() {
            tar::EntryType::Regular => {
                let mut data = Vec::new();
                entry
                    .read_to_end(&mut data)
                    .map_err(|cause| cause.to_string())?;
                let executable = header.mode().is_ok_and(|mode| mode & 0o111 != 0);
                tree.insert(path, Node::File { data, executable });
            }
            tar::EntryType::Symlink => {
                let target = header
                    .link_name()
                    .ok()
                    .flatten()
                    .map(|target| target.to_string_lossy().to_string())
                    .unwrap_or_default();
                tree.insert(path, Node::Link(target));
            }
            // Folders come with the files in them; the updater writes each file's parents.
            _ => {}
        }
    }
    Ok((tree, (control.clone(), control_data.to_vec())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program_files() -> Vec<u8> {
        tarball(
            &[
                ("opt/KChess/kchess".into(), vec![7u8; 300_000], 0o755),
                ("opt/KChess/notes.txt".into(), b"odd".to_vec(), 0o644),
            ],
            &["opt".into(), "opt/KChess".into()],
        )
        .unwrap()
    }

    /// A `.deb` reads back as the files written into it, however its data is compressed; folders
    /// come with their files.
    #[test]
    fn a_deb_reads_back_as_written() {
        let tar = program_files();
        let control = gzip(
            &tarball(
                &[("control".into(), b"Package: kchess\n".to_vec(), 0o644)],
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        let packed = xz(&tar).unwrap();
        // xz as dpkg reads it, far smaller than the program files.
        assert!(packed.starts_with(b"\xfd7zXZ\0"));
        assert!(packed.len() < tar.len() / 10);
        assert!(control.starts_with(&[0x1f, 0x8b]));
        for (member, data) in [
            ("data.tar.xz", packed),
            ("data.tar.gz", gzip(&tar).unwrap()),
            ("data.tar", tar.clone()),
        ] {
            let deb = ar(&[
                ("debian-binary", b"2.0\n"),
                ("control.tar.gz", &control),
                (member, &data),
            ]);
            let (tree, found) = read(&deb).unwrap();
            assert_eq!(
                found,
                ("control.tar.gz".to_string(), control.clone()),
                "{member}"
            );
            assert_eq!(
                tree.get("opt/KChess/kchess"),
                Some(&Node::File {
                    data: vec![7u8; 300_000],
                    executable: true
                })
            );
            assert_eq!(
                tree.get("opt/KChess/notes.txt"),
                Some(&Node::File {
                    data: b"odd".to_vec(),
                    executable: false
                })
            );
            assert_eq!(tree.len(), 2, "{member}");
        }
    }

    /// A link in the data archive reads as a link (the writer writes none; a `.deb` another tool
    /// made may).
    #[test]
    fn a_link_in_a_deb_is_read_as_a_link() {
        let mut archive = tar::Builder::new(Vec::new());
        let mut link = tar::Header::new_gnu();
        link.set_path("./usr/bin/kchess").unwrap();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_link_name("/opt/KChess/kchess").unwrap();
        link.set_size(0);
        link.set_cksum();
        archive.append(&link, std::io::empty()).unwrap();
        let tar = archive.into_inner().unwrap();
        let (tree, _) = read(&ar(&[("control.tar.gz", b"c"), ("data.tar", &tar)])).unwrap();
        assert_eq!(
            tree.get("usr/bin/kchess"),
            Some(&Node::Link("/opt/KChess/kchess".into()))
        );
    }

    #[test]
    fn ar_members_are_padded_to_even_lengths() {
        let archive = ar(&[("debian-binary", b"2.0\n"), ("odd", b"abc"), ("next", b"x")]);
        assert!(archive.starts_with(
            b"!<arch>\ndebian-binary   0           0     0     100644  4         `\n2.0\n"
        ));
        let members = ar_members(&archive).unwrap();
        let names: Vec<(&str, &[u8])> = members
            .iter()
            .map(|(name, data)| (name.as_str(), *data))
            .collect();
        assert_eq!(
            names,
            [
                ("debian-binary", &b"2.0\n"[..]),
                ("odd", b"abc"),
                ("next", b"x")
            ]
        );
    }

    #[test]
    fn a_broken_deb_is_refused() {
        let tar = program_files();
        assert!(read(b"not an archive").is_err());
        assert!(read(&ar(&[("debian-binary", b"2.0\n")])).is_err());
        assert!(read(&ar(&[("control.tar.gz", b"c"), ("data.tar.zst", b"d")])).is_err());
        let whole = ar(&[("control.tar.gz", b"c"), ("data.tar", &tar)]);
        assert!(read(&whole[..whole.len() - 100]).is_err(), "cut short");
    }
}
