//! Releases: the next version (`kchess_delta::Version`), its notes, and the checks a release
//! commit and its tag pass. The workspace version in Cargo.toml is the release.

use kchess_delta::notes::{self, FILE as NOTES, UNRELEASED};
use kchess_delta::{Asset, Version};

use crate::util::{Result, output, root, run, say, version, workspace_version};

/// `version` as a release version, refused unless it is one.
pub fn parse(version: &str) -> Result<Version> {
    Version::parse(version).ok_or_else(|| {
        format!(
            "Invalid release version {version}: use YEAR.MONTH.COUNTER, without zero padding (e.g. 2026.10.0)."
        )
    })
}

/// The next release in `(year, month)` after `tags`.
pub fn next_version(tags: &[&str], year: u32, month: u32) -> Result<String> {
    let mut counter: Option<u64> = None;
    for tag in tags {
        let Some(version) = tag.strip_prefix('v') else {
            continue;
        };
        let Some(Version {
            year: tag_year,
            month: tag_month,
            counter: tag_counter,
        }) = Version::parse(version)
        else {
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

fn utc_date() -> (u32, u32, u32) {
    let (year, month, day) = crate::util::civil_date(crate::util::unix_now());
    (year as u32, month, day)
}

/// `notes` with its unreleased items as `version`'s, released on `date` (`YYYY-MM-DD`), under a
/// fresh, empty `## Unreleased`. Refused when nothing is written there: the app would show an
/// update with no notes.
pub fn date_notes(notes: &str, version: &str, date: &str) -> Result<String> {
    let unreleased = format!("## {UNRELEASED}");
    let missing = || {
        format!(
            "{NOTES} has nothing under {unreleased}. Write what this release brings there, one \
             `- **Title**: what it does` line each, then run release-version again."
        )
    };
    // The heading line itself, not a mention of it in the file's introduction.
    let (head, rest) = notes
        .split_once(&format!("\n{unreleased}\n"))
        .map(|(head, rest)| (format!("{head}\n"), format!("\n{rest}")))
        .ok_or_else(missing)?;
    let section = rest.split("\n## ").next().unwrap_or_default();
    if !section
        .lines()
        .any(|line| line.trim_start().starts_with("- "))
    {
        return Err(missing());
    }
    if has_notes(notes, version) {
        return Err(format!("{NOTES} already has a section for {version}."));
    }
    Ok(format!("{head}{unreleased}\n\n## {version} ({date}){rest}"))
}

/// The items of `version`'s section in `notes`, as the release's text; none when it has none.
pub fn release_notes(notes: &str, version: &str) -> Option<String> {
    let body = notes::section(notes, version)?.body();
    (!body.is_empty()).then_some(body)
}

/// Whether `notes` has a heading for `version`, even one with nothing under it.
pub fn has_notes(notes: &str, version: &str) -> bool {
    notes::section(notes, version).is_some()
}

/// Cargo.toml's text with `next` as the workspace version, and no other version changed.
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
    let (year, month, day) = utc_date();
    let next = next_version(&tags.lines().collect::<Vec<_>>(), year, month)?;
    // The notes first: refused, nothing has changed.
    let notes_path = root().join(NOTES);
    let notes = std::fs::read_to_string(&notes_path).map_err(|cause| cause.to_string())?;
    let notes = date_notes(&notes, &next, &format!("{year}-{month:02}-{day:02}"))?;
    let path = root().join("Cargo.toml");
    let manifest = std::fs::read_to_string(&path).map_err(|cause| cause.to_string())?;
    std::fs::write(&path, with_version(&manifest, &next)?).map_err(|cause| cause.to_string())?;
    std::fs::write(&notes_path, notes).map_err(|cause| cause.to_string())?;
    // The workspace crates' versions in Cargo.lock follow.
    run("cargo", &["update", "--workspace", "--offline"])?;
    say(&format!(
        "Cargo.toml is now {next}. Review and stage Cargo.toml, Cargo.lock and {NOTES}, run `cargo xtask check`,\n\
         then commit the release change to main. The tag must be v{next}, pointing at that commit on\n\
         origin/main with green CI. Pushing the tag creates a draft release; publishing is separate."
    ));
    Ok(())
}

/// Whether `lock` gives the app the workspace `version`, whichever line endings its checkout has
/// (`core.autocrlf` writes Cargo.lock with CRLF on Windows).
fn lock_names_version(lock: &str, version: &str) -> bool {
    let mut lines = lock.lines();
    let wanted = format!("version = \"{version}\"");
    while let Some(line) = lines.next() {
        if line == "name = \"kchess-gpui\"" {
            return lines.next() == Some(wanted.as_str());
        }
    }
    false
}

/// `release-verify`: the version is a release version, and a tag build's tag names it exactly.
pub fn verify() -> Result {
    let version = version()?;
    let lock =
        std::fs::read_to_string(root().join("Cargo.lock")).map_err(|cause| cause.to_string())?;
    let notes = std::fs::read_to_string(root().join(NOTES)).map_err(|cause| cause.to_string())?;
    metadata(&version, &lock, &notes, tag_build().as_deref())?;
    say(&format!("Release metadata verified: v{version}"));
    Ok(())
}

/// Whether release `version` has its Cargo.lock, its notes and, on a tag build, its tag.
fn metadata(version: &str, lock: &str, notes: &str, tag: Option<&str>) -> Result {
    parse(version)?;
    if !lock_names_version(lock, version) {
        return Err(
            "Cargo.lock does not match the workspace version. Run `cargo update --workspace`."
                .into(),
        );
    }
    if !has_notes(notes, version) {
        return Err(format!(
            "{NOTES} has no `## {version} (date)` section, so the app would show this release \
             with no notes. `cargo xtask release-version` writes it from `## {UNRELEASED}`."
        ));
    }
    if let Some(problem) = notes::problems(notes).into_iter().next() {
        return Err(format!("{NOTES}: {problem}"));
    }
    // The App Store takes the same notes: one it would refuse is rewritten before the tag.
    crate::store_notes::plain(&release_notes(notes, version).unwrap_or_default())?;
    if let Some(tag) = tag
        && tag != format!("v{version}")
    {
        return Err(format!("Tag {tag} must be exactly v{version}."));
    }
    Ok(())
}

/// The tag a GitHub Actions run builds (`GITHUB_REF` is `refs/tags/<tag>`); none for a branch,
/// a pull request or a run outside Actions.
fn tag_build() -> Option<String> {
    tag_of(&std::env::var("GITHUB_REF").ok()?).map(str::to_string)
}

/// The tag a Git reference names (`refs/tags/<tag>`).
fn tag_of(reference: &str) -> Option<&str> {
    reference.strip_prefix("refs/tags/")
}

/// Whether `release-verify` needs the commit's CI: run by hand, or by a tag build, not by a manual
/// Release build of a branch in Actions (downloads only).
fn needs_ci(in_actions: bool, tag: Option<&str>) -> bool {
    !in_actions || tag.is_some()
}

/// The tag a tag-push run builds, from the event's name and `GITHUB_REF`: none for any other run.
fn pushed_tag<'a>(event: Option<&str>, reference: Option<&'a str>) -> Option<&'a str> {
    reference.and_then(tag_of).filter(|_| event == Some("push"))
}

/// The version a release tag names (`v2026.11.5` → `2026.11.5`), refused unless it is one.
pub fn tag_version(tag: &str) -> Result<&str> {
    let version = tag
        .strip_prefix('v')
        .ok_or_else(|| format!("The release tag {tag:?} must be v and the version."))?;
    parse(version)?;
    Ok(version)
}

/// `release-verify`: the metadata, and CI green for the commit with every job run. On a push to
/// main the packages, the Snap and the Windows upgrade run only for a release commit (ci.yml), so
/// a release commit has had them all; before, each of them first failed after a tag. A manual Release build (downloads only) skips this.
pub fn verify_release() -> Result {
    verify()?;
    if !needs_ci(
        std::env::var_os("GITHUB_ACTIONS").is_some(),
        tag_build().as_deref(),
    ) {
        say("Not a tag: CI is not required for a manual Release build.");
        return Ok(());
    }
    crate::update_key::require_public_key()?;
    let commit = output("git", &["rev-parse", "HEAD"])?.trim().to_string();
    let run = output(
        "gh",
        &[
            "run",
            "list",
            "--workflow",
            "ci.yml",
            "--commit",
            &commit,
            "--event",
            "push",
            "--limit",
            "1",
            "--json",
            "databaseId,status,conclusion",
            "-q",
            ".[]|\"\\(.databaseId)\\t\\(.status)\\t\\(.conclusion)\"",
        ],
    )?;
    let id = run
        .split('\t')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    let jobs = if id.is_empty() {
        String::new()
    } else {
        output(
            "gh",
            &[
                "run",
                "view",
                &id,
                "--json",
                "jobs",
                "-q",
                ".jobs[]|\"\\(.conclusion)\\t\\(.name)\"",
            ],
        )?
    };
    ci_verdict(&commit, &run, &jobs)?;
    say(&format!("CI passed every job for {commit}."));
    Ok(())
}

/// Whether CI's push run (`id\tstatus\tconclusion`) and its jobs (`conclusion\tname` lines) clear
/// `commit` for release.
pub fn ci_verdict(commit: &str, run: &str, jobs: &str) -> Result {
    let fields: Vec<&str> = run.trim().split('\t').collect();
    let [_, status, conclusion] = fields.as_slice() else {
        return Err(format!(
            "CI has not run for {commit}: push it to main and wait for CI."
        ));
    };
    if *status != "completed" {
        return Err(format!("CI for {commit} is still {status}."));
    }
    if *conclusion != "success" {
        return Err(format!(
            "CI for {commit} ended {conclusion}; a later push cancels it, so re-run it."
        ));
    }
    let not_run: Vec<&str> = jobs
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(conclusion, _)| *conclusion != "success")
        .map(|(_, name)| name)
        .collect();
    if !not_run.is_empty() {
        return Err(format!(
            "CI for {commit} did not run and pass: {}. A release commit changes Cargo.toml, which runs them.",
            not_run.join(", ")
        ));
    }
    Ok(())
}

/// `attach-draft-release <dir>`: a tag-push build attaches every file in `dir` to the tag's
/// draft release, creating it as a draft. A published release is never changed: its assets are
/// permanent, so a fix ships as the next version.
pub fn attach_draft(dir: &std::path::Path) -> Result {
    let event = std::env::var("GITHUB_EVENT_NAME").ok();
    let reference = std::env::var("GITHUB_REF").ok();
    let Some(tag) = pushed_tag(event.as_deref(), reference.as_deref()).map(str::to_string) else {
        return Err(
            "Draft releases may only be attached by a tag-push workflow, never a manual build."
                .into(),
        );
    };
    let version = tag_version(&tag)?;
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .map_err(|cause| cause.to_string())?
        .flatten()
        .map(|entry| entry.path().to_string_lossy().to_string())
        .collect();
    files.sort();
    if files.is_empty() {
        return Err("No release assets were provided.".into());
    }
    if let Some(store) = files.iter().find(|file| store_only(file)) {
        return Err(format!(
            "{store} is the App Store package; it goes to App Store Connect, not a GitHub release."
        ));
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
            let title = format!("KChess {version}");
            // The notes the app shows for this release, reviewed in the release commit, are the
            // release's text, and through it the App Store's What's New.
            let notes = std::fs::read_to_string(root().join(NOTES))
                .ok()
                .and_then(|notes| release_notes(&notes, version))
                .ok_or_else(|| format!("{NOTES} has no notes for {tag}."))?;
            let mut args = vec!["release", "create", tag.as_str()];
            args.extend(&file_args);
            // --verify-tag keeps gh from inventing a tag at the default branch's HEAD.
            args.extend([
                "--verify-tag",
                "--draft",
                "--notes",
                notes.as_str(),
                "--title",
                title.as_str(),
            ]);
            run("gh", &args)
        }
    }
}

/// The Mac App Store installer package, which only installs from the store: any `.pkg`, however
/// it is named.
fn store_only(file: &str) -> bool {
    file.ends_with(".pkg")
}

/// The release's Snap, named for its tag, as SHA256SUMS.txt lists it exactly once.
pub fn verify_snap(tag: &str, checksums: &str, digest: &str) -> Result<String> {
    let name = Asset::Snap.released(tag_version(tag)?);
    if crate::sums::listed(checksums, &name) != [digest] {
        return Err("Snap checksum is missing, ambiguous or incorrect.".into());
    }
    Ok(name)
}

/// `download-release-snap`: the published release's Snap into `packages/`, checked against its
/// SHA256SUMS.txt; only the latest release may reach the store's stable channel.
pub fn download_snap() -> Result {
    let tag = std::env::var("RELEASE_TAG").unwrap_or_default();
    let name = Asset::Snap.released(tag_version(&tag)?);
    let latest: serde_json::Value = serde_json::from_str(&output(
        "gh",
        &[
            "api",
            &format!("repos/{}/releases/latest", crate::util::repository()),
        ],
    )?)
    .map_err(|cause| cause.to_string())?;
    if latest["tag_name"] != tag.as_str() {
        return Err("Refusing to publish an older release over latest/stable.".into());
    }
    run(
        "gh",
        &[
            "release",
            "download",
            &tag,
            "--pattern",
            &name,
            "--pattern",
            crate::sums::FILE,
            "--dir",
            "packages",
        ],
    )?;
    let checksums = std::fs::read_to_string(root().join("packages").join(crate::sums::FILE))
        .map_err(|cause| cause.to_string())?;
    let digest = crate::sums::file(&root().join("packages").join(&name))?;
    verify_snap(&tag, &checksums, &digest).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Windows checkout with `core.autocrlf` has Cargo.lock in CRLF, and `check` refused it.
    #[test]
    fn the_lock_matches_with_either_line_ending() {
        let lock = "[[package]]\nname = \"kchess-gpui\"\nversion = \"2026.11.1\"\n";
        assert!(lock_names_version(lock, "2026.11.1"));
        assert!(lock_names_version(&lock.replace('\n', "\r\n"), "2026.11.1"));
        assert!(!lock_names_version(lock, "2026.11.2"));
        assert!(!lock_names_version(
            "name = \"kchess-core\"\nversion = \"2026.11.1\"\n",
            "2026.11.1"
        ));
    }

    #[test]
    fn release_versions_have_no_padding() {
        assert_eq!(parse("2026.10.0").unwrap().to_string(), "2026.10.0");
        assert_eq!(parse("2026.1.12").unwrap().to_string(), "2026.1.12");
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
        let (abc, def) = ("abc".repeat(21) + "a", "d".repeat(64));
        let sums = format!("{abc}  {name}\n{def}  other.deb\n");
        assert_eq!(verify_snap("v2026.10.16", &sums, &abc).unwrap(), name);
        assert!(verify_snap("v2026.10.16", &sums, &def).is_err());
        assert!(verify_snap("v2026.10.16", &format!("{sums}{abc}  {name}\n"), &abc).is_err());
        assert!(verify_snap("2026.10.16", &sums, &abc).is_err());
    }

    #[test]
    fn a_release_needs_every_ci_job_run_and_passed() {
        let jobs = "success\tcheck\nsuccess\tsnap\nsuccess\twindows-upgrade / Upgrade (allusers)\n";
        assert!(ci_verdict("abc", "1\tcompleted\tsuccess\n", jobs).is_ok());
        // Never pushed, still running, or cancelled by a later push.
        assert!(ci_verdict("abc", "", "").is_err());
        assert!(ci_verdict("abc", "1\tin_progress\t", "").is_err());
        assert!(ci_verdict("abc", "1\tcompleted\tcancelled", jobs).is_err());
        // Green, but the packaging jobs were skipped: nothing packaging-related changed.
        let skipped = "success\tcheck\nskipped\tsnap\nskipped\tpackage (macos-latest)\n";
        let refused = ci_verdict("abc", "1\tcompleted\tsuccess", skipped).unwrap_err();
        assert!(
            refused.contains("snap, package (macos-latest)"),
            "{refused}"
        );
    }

    #[test]
    fn the_app_store_package_is_never_a_github_download() {
        assert!(store_only("packages/KChess-2026.11.3-mac-arm64.pkg"));
        for asset in [
            "KChess-2026.11.3-mac-arm64.zip",
            "KChess-2026.11.3-mac-arm64.dmg",
            "KChess-2026.11.3-win-x64.exe",
            "KChess-2026.11.3-linux-amd64.deb",
            "SHA256SUMS.txt",
        ] {
            assert!(!store_only(asset), "{asset}");
        }
    }

    #[test]
    fn a_release_needs_its_lock_notes_and_tag() {
        let lock = "name = \"kchess-gpui\"\nversion = \"2026.11.5\"\n";
        let notes = "## Unreleased\n\n## 2026.11.5 (2026-11-08)\n\n- **New**: a thing.\n";
        assert_eq!(metadata("2026.11.5", lock, notes, None), Ok(()));
        assert_eq!(
            metadata("2026.11.5", lock, notes, Some("v2026.11.5")),
            Ok(())
        );
        assert!(
            metadata("2026.11.5", lock, notes, Some("v2026.11.4"))
                .unwrap_err()
                .contains("must be exactly v2026.11.5")
        );
        assert!(
            metadata("2026.11.5", &lock.replace(".5", ".4"), notes, None)
                .unwrap_err()
                .contains("Cargo.lock")
        );
        assert!(
            metadata("2026.11.5", lock, "## Unreleased\n", None)
                .unwrap_err()
                .contains("no `## 2026.11.5 (date)` section")
        );
        assert!(metadata("1.2.3", lock, notes, None).is_err());
        // A line too long for a player to read is caught before the tag too.
        let long = notes.replace("a thing", &"a long thing ".repeat(30));
        assert!(
            metadata("2026.11.5", lock, &long, None)
                .unwrap_err()
                .contains("over 280")
        );
        // Notes App Store Connect would refuse are caught before the tag.
        assert!(
            metadata("2026.11.5", lock, &notes.replace("thing", "thing 🎉"), None)
                .unwrap_err()
                .contains("App Store Connect refuses")
        );
    }

    /// Run by hand or for a tag, the release check needs CI; a manual Release build of a branch
    /// does not. Only a tag push attaches a draft.
    #[test]
    fn ci_and_drafts_are_for_tag_builds() {
        assert!(needs_ci(false, None));
        assert!(needs_ci(true, Some("v2026.11.5")));
        assert!(!needs_ci(true, None));
        let tag = Some("refs/tags/v2026.11.5");
        assert_eq!(pushed_tag(Some("push"), tag), Some("v2026.11.5"));
        assert_eq!(pushed_tag(Some("workflow_dispatch"), tag), None);
        assert_eq!(pushed_tag(Some("push"), Some("refs/heads/main")), None);
        assert_eq!(pushed_tag(None, tag), None);
    }

    #[test]
    fn only_a_tag_reference_is_a_tag_build() {
        assert_eq!(tag_of("refs/tags/v2026.11.5"), Some("v2026.11.5"));
        assert_eq!(tag_of("refs/heads/main"), None);
        assert_eq!(tag_of("refs/pull/7/merge"), None);
    }

    #[test]
    fn a_release_tag_is_v_and_a_release_version() {
        assert_eq!(tag_version("v2026.11.5"), Ok("2026.11.5"));
        for bad in ["2026.11.5", "v2026.11.05", "v1.2.3", "", "v"] {
            assert!(tag_version(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_release_dates_the_unreleased_notes() {
        let notes = "# What's new\n\nAdd lines under ## Unreleased below.\n\n## Unreleased\n\n- **New**: a thing.\n\n## 2026.11.3 (2026-11-04)\n\n- **Old**: before.\n";
        let dated = date_notes(notes, "2026.11.4", "2026-11-20").unwrap();
        assert_eq!(
            dated,
            "# What's new\n\nAdd lines under ## Unreleased below.\n\n## Unreleased\n\n## 2026.11.4 (2026-11-20)\n\n- **New**: a thing.\n\n## 2026.11.3 (2026-11-04)\n\n- **Old**: before.\n"
        );
        assert!(has_notes(&dated, "2026.11.4"));
        assert!(has_notes(&dated, "2026.11.3"));
        assert!(!has_notes(&dated, "2026.11.5"));
        assert_eq!(
            release_notes(&dated, "2026.11.4").as_deref(),
            Some("- **New**: a thing.")
        );
        assert_eq!(
            release_notes(&dated, "2026.11.3").as_deref(),
            Some("- **Old**: before.")
        );
        assert_eq!(release_notes(&dated, "2026.11.5"), None);
        // A heading with nothing under it is a section, but no release text.
        let empty = "## 2026.11.6 (2026-11-22)\n\n## 2026.11.5 (2026-11-21)\n\n- **Fixes**: x.\n";
        assert!(has_notes(empty, "2026.11.6"));
        assert_eq!(release_notes(empty, "2026.11.6"), None);
        // Nothing new to say: no release yet.
        assert!(date_notes(&dated, "2026.11.5", "2026-11-21").is_err());
        assert!(date_notes("# What's new\n", "2026.11.4", "2026-11-20").is_err());
    }
}
