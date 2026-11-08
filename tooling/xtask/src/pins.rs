//! `cargo xtask pins --check`: whether the AppImage tools the Linux package pins
//! (`package/linux.rs`) still match the newest upstream releases. When one is behind it prints
//! the pin and the newer release, writes them to `--report <file>` and fails. The weekly Pins
//! workflow (`.github/workflows/pins.yml`) runs it and opens one issue; `ci.yml` never does, so
//! an upstream release cannot turn a pull request red.

use std::path::Path;

use crate::package::linux::{APPIMAGE_RUNTIME, APPIMAGETOOL};
use crate::util::{Result, output, say};

/// A file the build downloads by a fixed release, and the constants that pin it.
struct Pin {
    name: &'static str,
    repo: &'static str,
    url: &'static str,
    url_constant: &'static str,
    digest_constant: &'static str,
}

const PINS: [Pin; 2] = [
    Pin {
        name: "the AppImage runtime",
        repo: "AppImage/type2-runtime",
        url: APPIMAGE_RUNTIME,
        url_constant: "APPIMAGE_RUNTIME",
        digest_constant: "APPIMAGE_RUNTIME_SHA256",
    },
    Pin {
        name: "appimagetool",
        repo: "AppImage/appimagetool",
        url: APPIMAGETOOL,
        url_constant: "APPIMAGETOOL",
        digest_constant: "APPIMAGETOOL_SHA256",
    },
];

/// The newest release upstream: its tag, page and the digest GitHub records for the pinned file.
#[derive(Debug, PartialEq)]
struct Release {
    tag: String,
    page: String,
    digest: Option<String>,
}

/// The release tag in a GitHub download URL (`…/releases/download/<tag>/<file>`).
fn release_tag(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("/releases/download/")?;
    let (tag, file) = rest.split_once('/')?;
    (!tag.is_empty() && !file.is_empty()).then_some(tag)
}

/// The file name a download URL ends with.
fn file_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

/// A release tag as numbers to compare (`1.9.1`, `v1.9.1`, `20251108`); moving or named tags
/// (`continuous`, `old`) are not versions.
fn version(tag: &str) -> Option<Vec<u64>> {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    tag.split('.')
        .map(|part| {
            (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        })
        .collect()
}

/// The newest published, versioned release in GitHub's answer to `repos/<repo>/releases`.
fn newest(answer: &str, file: &str) -> Result<Release> {
    let unreadable = |why: &str| format!("GitHub's list of releases is unreadable: {why}.");
    let releases: serde_json::Value =
        serde_json::from_str(answer).map_err(|cause| unreadable(&cause.to_string()))?;
    let releases = releases
        .as_array()
        .ok_or_else(|| unreadable("it is not a list"))?;
    let flag = |release: &serde_json::Value, key: &str| release[key].as_bool().unwrap_or(false);
    releases
        .iter()
        .filter(|release| !flag(release, "draft") && !flag(release, "prerelease"))
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            Some((version(tag)?, tag, release))
        })
        .max_by(|(a, ..), (b, ..)| a.cmp(b))
        .map(|(_, tag, release)| Release {
            tag: tag.to_string(),
            page: release["html_url"].as_str().unwrap_or_default().to_string(),
            digest: release["assets"].as_array().and_then(|assets| {
                assets
                    .iter()
                    .find(|asset| asset["name"].as_str() == Some(file))
                    .and_then(|asset| asset["digest"].as_str())
                    .map(|digest| digest.trim_start_matches("sha256:").to_string())
            }),
        })
        .ok_or_else(|| unreadable("it names no versioned release"))
}

/// The newer release when `newest` is ahead of the pinned `tag`.
fn newer(tag: &str, newest: Release) -> Result<Option<Release>> {
    let pinned = version(tag).ok_or_else(|| format!("The pinned release {tag} is no version."))?;
    let latest = version(&newest.tag).unwrap_or_default();
    Ok((latest > pinned).then_some(newest))
}

/// The issue text naming each pin that is behind and the constants to change.
fn report(behind: &[(&Pin, &str, Release)]) -> String {
    let mut text = String::from("Newer releases of the AppImage tools KChess pins are out:\n\n");
    for (pin, tag, release) in behind {
        text += &format!(
            "- **{}** (`{}`): pinned `{tag}`, newest [`{}`]({}).\n",
            pin.name, pin.repo, release.tag, release.page
        );
        if let Some(digest) = &release.digest {
            text += &format!("  Its `{}` has SHA-256 `{digest}`.\n", file_name(pin.url));
        }
        text += &format!(
            "  Change `{}` and `{}` in `tooling/xtask/src/package/linux.rs`.\n",
            pin.url_constant, pin.digest_constant
        );
    }
    text += "\nUpdate a pin in a pull request, checking the downloaded file against its digest: \
             CI then packages and tests the AppImage with it. `cargo xtask pins --check` \
             answers whether the pins are current.\n";
    text
}

/// The report when a pin is behind, asking `releases` for each repository's releases.
fn check(releases: impl Fn(&str) -> Result<String>) -> Result<Option<String>> {
    let mut behind = Vec::new();
    for pin in &PINS {
        let tag = release_tag(pin.url)
            .ok_or_else(|| format!("{} names no release: {}", pin.url_constant, pin.url))?;
        let latest = newest(&releases(pin.repo)?, file_name(pin.url))
            .map_err(|cause| format!("{} ({}): {cause}", pin.name, pin.repo))?;
        match newer(tag, latest)? {
            Some(release) => behind.push((pin, tag, release)),
            None => say(&format!("{} is at the newest release, {tag}.", pin.name)),
        }
    }
    Ok((!behind.is_empty()).then(|| report(&behind)))
}

/// The file `--report` names, once `--check` is given.
fn report_file(args: &[String]) -> Result<Option<&String>> {
    if !args.iter().any(|arg| arg == "--check") {
        return Err("pins needs --check.".into());
    }
    match args.iter().position(|arg| arg == "--report") {
        Some(at) => args
            .get(at + 1)
            .map(Some)
            .ok_or_else(|| "--report needs the file to write.".to_string()),
        None => Ok(None),
    }
}

pub fn run(args: &[String]) -> Result {
    let report_file = report_file(args)?;
    let releases = |repo: &str| {
        output(
            "gh",
            &["api", &format!("repos/{repo}/releases?per_page=100")],
        )
    };
    let Some(text) = check(releases)? else {
        return Ok(());
    };
    print!("{text}");
    if let Some(path) = report_file {
        std::fs::write(Path::new(path), &text)
            .map_err(|cause| format!("Cannot write {path}: {cause}"))?;
    }
    Err("A pinned AppImage tool is behind its newest release.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/o/r/releases/tag/{tag}"),
            "draft": false,
            "prerelease": false,
            "assets": [{"name": "runtime-x86_64", "digest": format!("sha256:{tag}digest")}],
        })
    }

    fn answer(releases: &[serde_json::Value]) -> String {
        serde_json::Value::Array(releases.to_vec()).to_string()
    }

    #[test]
    fn the_pins_name_their_release_tags() {
        assert_eq!(release_tag(APPIMAGE_RUNTIME), Some("20251108"));
        assert_eq!(release_tag(APPIMAGETOOL), Some("1.9.0"));
        assert_eq!(file_name(APPIMAGE_RUNTIME), "runtime-x86_64");
        assert_eq!(release_tag("https://example.test/runtime"), None);
        assert_eq!(release_tag("https://x/releases/download//runtime"), None);
        assert_eq!(release_tag("https://x/releases/download/1.0/"), None);
    }

    #[test]
    fn release_tags_compare_as_versions() {
        assert_eq!(version("20251108"), Some(vec![20251108]));
        assert_eq!(version("v1.9.1"), Some(vec![1, 9, 1]));
        assert_eq!(version("continuous"), None);
        assert_eq!(version("1.9-beta"), None);
        assert_eq!(version("1..9"), None);
        // `+1` parses as a number but is no release tag.
        assert_eq!(version("1.+9"), None);
        assert!(version("1.10.0") > version("1.9.1"));
    }

    #[test]
    fn the_newest_release_skips_moving_tags_drafts_and_prereleases() {
        let mut draft = release("1.11.0");
        draft["draft"] = true.into();
        let mut beta = release("1.10.0");
        beta["prerelease"] = true.into();
        let text = answer(&[
            release("continuous"),
            release("1.9.0"),
            release("1.9.1"),
            draft,
            beta,
            release("old"),
        ]);
        assert_eq!(
            newest(&text, "runtime-x86_64"),
            Ok(Release {
                tag: "1.9.1".into(),
                page: "https://github.com/o/r/releases/tag/1.9.1".into(),
                digest: Some("1.9.1digest".into()),
            })
        );
        assert_eq!(newest(&text, "other").unwrap().digest, None);
    }

    #[test]
    fn an_unreadable_answer_is_an_error() {
        for text in [
            "<html>504 Gateway Time-out</html>",
            r#"{"message": "API rate limit exceeded"}"#,
            "[]",
            &answer(&[release("continuous")]),
        ] {
            let error = newest(text, "runtime-x86_64").unwrap_err();
            assert!(
                error.starts_with("GitHub's list of releases is unreadable"),
                "{error}"
            );
        }
    }

    #[test]
    fn only_a_newer_release_is_reported() {
        let latest = |tag: &str| newest(&answer(&[release(tag)]), "runtime-x86_64").unwrap();
        assert_eq!(newer("20251108", latest("20250101")), Ok(None));
        assert_eq!(newer("20251108", latest("20251108")), Ok(None));
        assert_eq!(
            newer("20251108", latest("20260301"))
                .unwrap()
                .map(|r| r.tag),
            Some("20260301".into())
        );
        assert!(newer("continuous", latest("20260301")).is_err());
    }

    #[test]
    fn the_check_reports_each_pin_that_is_behind() {
        let current = |repo: &str| -> Result<String> {
            Ok(answer(&[match repo {
                "AppImage/type2-runtime" => release("20251108"),
                _ => release("1.9.0"),
            }]))
        };
        assert_eq!(check(current), Ok(None));

        let runtime_newer = |repo: &str| -> Result<String> {
            Ok(answer(&[match repo {
                "AppImage/type2-runtime" => release("20260301"),
                _ => release("1.8.0"),
            }]))
        };
        let text = check(runtime_newer)
            .unwrap()
            .expect("the runtime is behind");
        assert!(text.contains(
            "- **the AppImage runtime** (`AppImage/type2-runtime`): pinned `20251108`, newest \
             [`20260301`](https://github.com/o/r/releases/tag/20260301).\n"
        ));
        assert!(text.contains("  Its `runtime-x86_64` has SHA-256 `20260301digest`.\n"));
        assert!(text.contains(
            "  Change `APPIMAGE_RUNTIME` and `APPIMAGE_RUNTIME_SHA256` in \
             `tooling/xtask/src/package/linux.rs`.\n"
        ));
        assert!(!text.contains("appimagetool**"), "{text}");

        let both_newer = |repo: &str| -> Result<String> {
            Ok(answer(&[match repo {
                "AppImage/type2-runtime" => release("20260301"),
                _ => release("1.9.1"),
            }]))
        };
        let text = check(both_newer).unwrap().unwrap();
        assert!(text.contains("pinned `1.9.0`, newest [`1.9.1`]"));
        assert!(text.contains("Change `APPIMAGETOOL` and `APPIMAGETOOL_SHA256`"));
        assert!(!text.contains("Its `appimagetool-x86_64.AppImage`"));
    }

    #[test]
    fn a_failed_or_unreadable_lookup_fails_the_check() {
        let down = |_: &str| -> Result<String> { Err("HTTP 504".into()) };
        assert_eq!(check(down), Err("HTTP 504".into()));
        let garbled = |_: &str| -> Result<String> { Ok("{}".into()) };
        let error = check(garbled).unwrap_err();
        assert!(error.starts_with("the AppImage runtime (AppImage/type2-runtime): GitHub's"));
    }

    #[test]
    fn the_command_needs_check_and_takes_a_report_file() {
        let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        assert_eq!(run(&[]), Err("pins needs --check.".into()));
        assert_eq!(
            report_file(&args(&["--report", "pins.md"])),
            Err("pins needs --check.".into())
        );
        assert_eq!(report_file(&args(&["--check"])), Ok(None));
        let given = args(&["--check", "--report", "pins.md"]);
        assert_eq!(report_file(&given), Ok(Some(&"pins.md".to_string())));
        assert_eq!(
            report_file(&args(&["--check", "--report"])),
            Err("--report needs the file to write.".into())
        );
    }
}
