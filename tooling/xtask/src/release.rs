//! Release versions: `YEAR.MONTH.COUNTER` in the UTC release month (2026.10.0, 2026.10.1, …
//! 2026.11.0), without zero padding, so the version stays valid semver for Cargo and the update
//! manifests. The workspace version in Cargo.toml is the release.

use crate::util::{Result, output, root, run, say, version, workspace_version};

/// `(year, month, counter)` of a release version.
pub fn parse(version: &str) -> Result<(u32, u32, u64)> {
    let invalid = || {
        format!(
            "Invalid release version {version}: use YEAR.MONTH.COUNTER, without zero padding (e.g. 2026.10.0)."
        )
    };
    let parts: Vec<&str> = version.split('.').collect();
    let [year, month, counter] = parts.as_slice() else {
        return Err(invalid());
    };
    let plain = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    if !(plain(year) && plain(month) && plain(counter))
        || year.len() != 4
        || !year.starts_with("20")
    {
        return Err(invalid());
    }
    let (year, month, counter) = (
        year.parse::<u32>().map_err(|_| invalid())?,
        month.parse::<u32>().map_err(|_| invalid())?,
        counter.parse::<u64>().map_err(|_| invalid())?,
    );
    if !(1..=12).contains(&month) {
        return Err(invalid());
    }
    Ok((year, month, counter))
}

/// The next release in `(year, month)` after `tags`.
pub fn next_version(tags: &[&str], year: u32, month: u32) -> Result<String> {
    let mut counter: Option<u64> = None;
    for tag in tags {
        let Some(version) = tag.strip_prefix('v') else {
            continue;
        };
        let Ok((tag_year, tag_month, tag_counter)) = parse(version) else {
            continue;
        };
        if (tag_year, tag_month) > (year, month) {
            return Err(format!(
                "Tag {tag} is ahead of the current UTC month. Check the clock and tags before releasing."
            ));
        }
        if (tag_year, tag_month) == (year, month) {
            counter = Some(counter.map_or(tag_counter, |known| known.max(tag_counter)));
        }
    }
    Ok(format!(
        "{year}.{month}.{}",
        counter.map_or(0, |known| known + 1)
    ))
}

/// The UTC year and month now, from the system clock (days since 1970 → civil date).
fn utc_month() -> (u32, u32) {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let days = (seconds / 86_400) as i64;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year as u32, month as u32)
}

/// Replace the workspace version in Cargo.toml's text.
pub fn with_version(manifest: &str, next: &str) -> Result<String> {
    let current = workspace_version(manifest).ok_or("Cargo.toml has no workspace version.")?;
    let from = format!("version = \"{current}\"");
    let (head, tail) = manifest
        .split_once("[workspace.package]")
        .ok_or("Cargo.toml has no [workspace.package].")?;
    let tail = tail.replacen(&from, &format!("version = \"{next}\""), 1);
    Ok(format!("{head}[workspace.package]{tail}"))
}

/// `release-version`: fetch origin's tags (required), then write the next version.
pub fn next() -> Result {
    if run("git", &["fetch", "origin", "--tags", "--quiet"]).is_err() {
        return Err(
            "Could not fetch release tags from origin. Version was not changed; restore access and retry."
                .into(),
        );
    }
    let tags = output("git", &["tag", "--list"])?;
    let (year, month) = utc_month();
    let next = next_version(&tags.lines().collect::<Vec<_>>(), year, month)?;
    let path = root().join("Cargo.toml");
    let manifest = std::fs::read_to_string(&path).map_err(|cause| cause.to_string())?;
    std::fs::write(&path, with_version(&manifest, &next)?).map_err(|cause| cause.to_string())?;
    // The workspace crates' versions in Cargo.lock follow.
    run("cargo", &["update", "--workspace", "--offline"])?;
    say(&format!(
        "Cargo.toml is now {next}. Review and stage Cargo.toml and Cargo.lock, run `cargo xtask check`,\n\
         then commit the release change to main. The tag must be v{next}, pointing at that commit on\n\
         origin/main with green CI. Pushing the tag creates a draft release; publishing is separate."
    ));
    Ok(())
}

/// `release-verify`: the version is a release version, and a tag build's tag names it exactly.
pub fn verify() -> Result {
    let version = version()?;
    parse(&version)?;
    let lock =
        std::fs::read_to_string(root().join("Cargo.lock")).map_err(|cause| cause.to_string())?;
    if !lock.contains(&format!("name = \"kchess-gpui\"\nversion = \"{version}\"")) {
        return Err(
            "Cargo.lock does not match the workspace version. Run `cargo update --workspace`."
                .into(),
        );
    }
    if std::env::var("GITHUB_REF").is_ok_and(|reference| reference.starts_with("refs/tags/")) {
        let tag = std::env::var("GITHUB_REF_NAME").unwrap_or_default();
        if tag != format!("v{version}") {
            return Err(format!("Tag {tag} must be exactly v{version}."));
        }
    }
    say(&format!("Release metadata verified: v{version}"));
    Ok(())
}

/// `attach-draft-release <dir>`: a tag-push build attaches every file in `dir` to the tag's
/// draft release, creating it as a draft. A published release is never changed: its assets are
/// permanent, so a fix ships as the next version.
pub fn attach_draft(dir: &std::path::Path) -> Result {
    let tag = std::env::var("GITHUB_REF_NAME").unwrap_or_default();
    if std::env::var("GITHUB_EVENT_NAME").as_deref() != Ok("push")
        || std::env::var("GITHUB_REF").as_deref() != Ok(&format!("refs/tags/{tag}"))
    {
        return Err(
            "Draft releases may only be attached by a tag-push workflow, never a manual build."
                .into(),
        );
    }
    parse(
        tag.strip_prefix('v')
            .ok_or("A release tag must start with v.")?,
    )?;
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .map_err(|cause| cause.to_string())?
        .flatten()
        .map(|entry| entry.path().to_string_lossy().to_string())
        .collect();
    files.sort();
    if files.is_empty() {
        return Err("No release assets were provided.".into());
    }
    let listed = output(
        "gh",
        &[
            "release",
            "list",
            "--limit",
            "1000",
            "--json",
            "tagName,isDraft",
        ],
    )?;
    let releases: serde_json::Value =
        serde_json::from_str(&listed).map_err(|_| "Could not verify existing releases.")?;
    let existing = releases
        .as_array()
        .ok_or("Could not verify existing releases.")?
        .iter()
        .find(|release| release["tagName"] == tag.as_str());
    let file_args: Vec<&str> = files.iter().map(String::as_str).collect();
    match existing {
        Some(release) if release["isDraft"] != true => Err(format!(
            "Release {tag} is already published. Advance the version and create a new tag; its assets will not be overwritten."
        )),
        Some(_) => {
            let mut args = vec!["release", "upload", tag.as_str()];
            args.extend(&file_args);
            args.push("--clobber");
            run("gh", &args)?;
            // --clobber only replaces: a draft retried with fewer files kept the ones no build made.
            let uploaded: Vec<String> = files
                .iter()
                .filter_map(|file| std::path::Path::new(file).file_name())
                .map(|name| name.to_string_lossy().to_string())
                .collect();
            let view = output("gh", &["release", "view", &tag, "--json", "assets"])?;
            let assets: serde_json::Value =
                serde_json::from_str(&view).map_err(|cause| cause.to_string())?;
            for asset in assets["assets"]
                .as_array()
                .ok_or("Could not list the draft release assets.")?
            {
                let name = asset["name"].as_str().unwrap_or_default();
                if !uploaded.iter().any(|kept| kept == name) {
                    run("gh", &["release", "delete-asset", &tag, name, "--yes"])?;
                }
            }
            Ok(())
        }
        None => {
            let title = format!("KChess {}", &tag[1..]);
            let mut args = vec!["release", "create", tag.as_str()];
            args.extend(&file_args);
            // --verify-tag keeps gh from inventing a tag at the default branch's HEAD.
            args.extend([
                "--verify-tag",
                "--draft",
                "--generate-notes",
                "--title",
                title.as_str(),
            ]);
            run("gh", &args)
        }
    }
}

/// The release's Snap, named for its tag, as SHA256SUMS.txt lists it exactly once.
pub fn verify_snap(tag: &str, checksums: &str, digest: &str) -> Result<String> {
    parse(tag.strip_prefix('v').ok_or("Missing release tag.")?)?;
    let name = format!("KChess-{}-linux-amd64.snap", &tag[1..]);
    let entries: Vec<&str> = checksums
        .lines()
        .filter(|line| line.ends_with(&format!("  {name}")))
        .collect();
    if entries.len() != 1 || entries[0] != format!("{digest}  {name}") {
        return Err("Snap checksum is missing, ambiguous or incorrect.".into());
    }
    Ok(name)
}

/// `download-release-snap`: the published release's Snap into `packages/`, checked against its
/// SHA256SUMS.txt; only the latest release may reach the store's stable channel.
pub fn download_snap() -> Result {
    let tag = std::env::var("RELEASE_TAG").unwrap_or_default();
    parse(tag.strip_prefix('v').ok_or("Missing release tag.")?)?;
    let repo = std::env::var("GH_REPO").unwrap_or_default();
    let latest: serde_json::Value = serde_json::from_str(&output(
        "gh",
        &["api", &format!("repos/{repo}/releases/latest")],
    )?)
    .map_err(|cause| cause.to_string())?;
    if latest["tag_name"] != tag.as_str() {
        return Err("Refusing to publish an older release over latest/stable.".into());
    }
    let name = format!("KChess-{}-linux-amd64.snap", &tag[1..]);
    run(
        "gh",
        &[
            "release",
            "download",
            &tag,
            "--pattern",
            &name,
            "--pattern",
            "SHA256SUMS.txt",
            "--dir",
            "packages",
        ],
    )?;
    let checksums = std::fs::read_to_string(root().join("packages/SHA256SUMS.txt"))
        .map_err(|cause| cause.to_string())?;
    let digest = crate::util::sha256_file(&root().join("packages").join(&name))?;
    verify_snap(&tag, &checksums, &digest).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_versions_have_no_padding() {
        assert_eq!(parse("2026.10.0").unwrap(), (2026, 10, 0));
        assert_eq!(parse("2026.1.12").unwrap(), (2026, 1, 12));
        for bad in [
            "2026.10.00",
            "2026.01.1",
            "2026.13.0",
            "v2026.10.0",
            "1.2.3",
            "2026.10",
            "2026.10.1-rc",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn counts_on_from_this_months_highest_tag() {
        let tags = [
            "v2026.9.4",
            "v2026.10.0",
            "v2026.10.15",
            "v2026.10.3",
            "not-a-tag",
            "v1.2.3",
        ];
        assert_eq!(next_version(&tags, 2026, 10).unwrap(), "2026.10.16");
        assert_eq!(next_version(&tags, 2026, 11).unwrap(), "2026.11.0");
        assert!(next_version(&tags, 2026, 9).unwrap_err().contains("ahead"));
    }

    #[test]
    fn writes_only_the_workspace_version() {
        let manifest = "[workspace]\n\n[workspace.package]\nversion = \"2026.10.15\"\n\n[dependencies]\nfoo = { version = \"2026.10.15\" }\n";
        let written = with_version(manifest, "2026.10.16").unwrap();
        assert!(written.contains("[workspace.package]\nversion = \"2026.10.16\""));
        assert!(written.contains("foo = { version = \"2026.10.15\" }"));
    }

    #[test]
    fn a_snap_is_published_only_with_its_one_checksum() {
        let name = "KChess-2026.10.16-linux-amd64.snap";
        let sums = format!("abc  {name}\ndef  other.deb\n");
        assert_eq!(verify_snap("v2026.10.16", &sums, "abc").unwrap(), name);
        assert!(verify_snap("v2026.10.16", &sums, "fff").is_err());
        assert!(verify_snap("v2026.10.16", &format!("{sums}abc  {name}\n"), "abc").is_err());
        assert!(verify_snap("2026.10.16", &sums, "abc").is_err());
    }

    #[test]
    fn the_clock_reads_a_month() {
        let (year, month) = utc_month();
        assert!(year >= 2026 && (1..=12).contains(&month));
    }
}
