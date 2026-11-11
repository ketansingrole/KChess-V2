//! `cargo xtask pins --check`: whether every pinned download (`pinned.rs`) still matches the newest
//! upstream release, or for a commit the branch's head. When one is behind it prints the pin and
//! the newer one, writes them to `--report <file>` and fails. The weekly Pins workflow
//! (`.github/workflows/pins.yml`) runs it and opens one issue; `ci.yml` never does, so an upstream
//! release cannot turn a pull request red.

use std::collections::HashMap;
use std::path::Path;

use crate::pinned::{self, Pin, Source};
use crate::util::{Result, output, say};

/// What upstream has that is newer than a pin: its version, page and, for a release, the digest
/// GitHub records for the pinned file under its new name.
#[derive(Debug, PartialEq)]
struct Newer {
    version: String,
    page: String,
    file: String,
    digest: Option<String>,
}

/// A release tag's numbers to compare (`1.9.1`, `v1.9.1`, `sf_19`, `20251108`); moving or named
/// tags (`continuous`, `old`) are not versions.
fn version(tag: &str) -> Option<Vec<u64>> {
    let tag = tag.trim_start_matches(|c: char| !c.is_ascii_digit());
    if tag.is_empty() {
        return None;
    }
    tag.split('.')
        .map(|part| {
            (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        })
        .collect()
}

/// The pinned file's name in release `tag`: the pin's version in it replaced by `tag`'s
/// (`vosk-osx-0.3.42.zip` in `v0.3.45` is `vosk-osx-0.3.45.zip`).
fn file_in(pin: &Pin, tag: &str) -> String {
    let digits = |tag: &'_ str| {
        tag.trim_start_matches(|c: char| !c.is_ascii_digit())
            .to_string()
    };
    let pinned = digits(pin.version);
    if pinned.is_empty() {
        return pin.file.to_string();
    }
    pin.file.replace(&pinned, &digits(tag))
}

/// The newest published, versioned release in GitHub's answer to `repos/<repo>/releases` that
/// carries `pin`'s file, when it is newer than the pin.
fn newer_release(answer: &str, pin: &Pin) -> Result<Option<Newer>> {
    let unreadable = |why: &str| format!("GitHub's list of releases is unreadable: {why}.");
    let releases: serde_json::Value =
        serde_json::from_str(answer).map_err(|cause| unreadable(&cause.to_string()))?;
    let releases = releases
        .as_array()
        .ok_or_else(|| unreadable("it is not a list"))?;
    let pinned = version(pin.version)
        .ok_or_else(|| format!("The pinned release {} is no version.", pin.version))?;
    let flag = |release: &serde_json::Value, key: &str| release[key].as_bool().unwrap_or(false);
    let versioned: Vec<_> = releases
        .iter()
        .filter(|release| !flag(release, "draft") && !flag(release, "prerelease"))
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            Some((version(tag)?, tag, release))
        })
        .collect();
    if versioned.is_empty() {
        return Err(unreadable("it names no versioned release"));
    }
    let asset = |release: &serde_json::Value, file: &str| {
        release["assets"].as_array().and_then(|assets| {
            assets
                .iter()
                .find(|asset| asset["name"].as_str() == Some(file))
                .cloned()
        })
    };
    Ok(versioned
        .into_iter()
        .filter(|(number, ..)| *number > pinned)
        .filter_map(|(number, tag, release)| {
            let file = file_in(pin, tag);
            let asset = asset(release, &file)?;
            Some((number, tag, release, file, asset))
        })
        .max_by(|(a, ..), (b, ..)| a.cmp(b))
        .map(|(_, tag, release, file, asset)| Newer {
            version: tag.to_string(),
            page: release["html_url"].as_str().unwrap_or_default().to_string(),
            file,
            digest: asset["digest"]
                .as_str()
                .map(|digest| digest.trim_start_matches("sha256:").to_string()),
        }))
}

/// The branch head in GitHub's answer to `repos/<repo>/commits/<branch>`, when it is not the
/// pinned commit.
fn newer_commit(answer: &str, pin: &Pin) -> Result<Option<Newer>> {
    let commit: serde_json::Value = serde_json::from_str(answer)
        .map_err(|cause| format!("GitHub's answer for the branch is unreadable: {cause}."))?;
    let sha = commit["sha"]
        .as_str()
        .ok_or("GitHub's answer for the branch names no commit.")?;
    Ok((sha != pin.version).then(|| Newer {
        version: sha.to_string(),
        page: commit["html_url"].as_str().unwrap_or_default().to_string(),
        file: pin.file.to_string(),
        digest: None,
    }))
}

/// The issue text naming each pin that is behind and the file to change it in.
fn report(behind: &[(Pin, &str, Newer)]) -> String {
    let mut text = String::from("Newer releases of the downloads KChess pins are out:\n\n");
    for (pin, file, newer) in behind {
        text += &format!(
            "- **{}** (`{}`, `{}`): pinned `{}`, newest [`{}`]({}).\n",
            pin.name, pin.repo, pin.file, pin.version, newer.version, newer.page
        );
        if let Some(digest) = &newer.digest {
            text += &format!("  Its `{}` has SHA-256 `{digest}`.\n", newer.file);
        }
        text += &format!("  Change the pin in `{file}`.\n");
    }
    text += "\nUpdate a pin in a pull request, checking the downloaded file against its digest: \
             CI then builds and tests with it. `cargo xtask pins --check` answers whether the \
             pins are current.\n";
    text
}

/// The report when a pin is behind, asking `github` for each API path (once per path).
fn check(
    pins: &[(Pin, &'static str)],
    github: impl Fn(&str) -> Result<String>,
) -> Result<Option<String>> {
    let mut answers: HashMap<String, String> = HashMap::new();
    let mut behind = Vec::new();
    for (pin, file) in pins {
        let path = match pin.source {
            Source::Release => format!("repos/{}/releases?per_page=100", pin.repo),
            Source::Commit { branch } => format!("repos/{}/commits/{branch}", pin.repo),
        };
        if !answers.contains_key(&path) {
            let answer = github(&path)?;
            answers.insert(path.clone(), answer);
        }
        let answer = answers.get(&path).map_or("", String::as_str);
        let newer = match pin.source {
            Source::Release => newer_release(answer, pin),
            Source::Commit { .. } => newer_commit(answer, pin),
        }
        .map_err(|cause| format!("{} ({}): {cause}", pin.name, pin.repo))?;
        match newer {
            Some(newer) => behind.push((*pin, *file, newer)),
            None => say(&format!(
                "{} ({}) is current at {}.",
                pin.name, pin.file, pin.version
            )),
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
    let github = |path: &str| output("gh", &["api", path]);
    let Some(text) = check(&pinned::all(), github)? else {
        return Ok(());
    };
    print!("{text}");
    if let Some(path) = report_file {
        std::fs::write(Path::new(path), &text)
            .map_err(|cause| format!("Cannot write {path}: {cause}"))?;
    }
    Err("A pinned download is behind its newest release.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNTIME: Pin = Pin {
        name: "the runtime",
        repo: "o/runtime",
        version: "20251108",
        file: "runtime-x86_64",
        sha256: "",
        source: Source::Release,
    };
    const VOSK: Pin = Pin {
        name: "libvosk",
        repo: "o/vosk",
        version: "v0.3.42",
        file: "vosk-osx-0.3.42.zip",
        sha256: "",
        source: Source::Release,
    };
    const ENGINE: Pin = Pin {
        name: "the engine",
        repo: "o/engine",
        version: "abc",
        file: "engine.tar.gz",
        sha256: "",
        source: Source::Commit { branch: "master" },
    };

    fn release(tag: &str, files: &[&str]) -> serde_json::Value {
        let assets: Vec<_> = files
            .iter()
            .map(|name| serde_json::json!({"name": name, "digest": format!("sha256:{tag}digest")}))
            .collect();
        serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/o/r/releases/tag/{tag}"),
            "draft": false,
            "prerelease": false,
            "assets": assets,
        })
    }

    fn answer(releases: &[serde_json::Value]) -> String {
        serde_json::Value::Array(releases.to_vec()).to_string()
    }

    #[test]
    fn release_tags_compare_as_versions() {
        assert_eq!(version("20251108"), Some(vec![20251108]));
        assert_eq!(version("v1.9.1"), Some(vec![1, 9, 1]));
        assert_eq!(version("sf_19"), Some(vec![19]));
        assert_eq!(version("continuous"), None);
        assert_eq!(version("1.9-beta"), None);
        assert_eq!(version("1..9"), None);
        // `+1` parses as a number but is no release tag.
        assert_eq!(version("1.+9"), None);
        assert!(version("1.10.0") > version("1.9.1"));
        // Every real pin's release is a version.
        for (pin, _) in pinned::all() {
            if pin.source == Source::Release {
                assert!(version(pin.version).is_some(), "{}", pin.name);
            }
        }
    }

    #[test]
    fn a_file_named_by_its_version_is_looked_for_under_the_new_one() {
        assert_eq!(file_in(&VOSK, "v0.3.45"), "vosk-osx-0.3.45.zip");
        assert_eq!(file_in(&RUNTIME, "20260301"), "runtime-x86_64");
    }

    #[test]
    fn the_newest_release_skips_moving_tags_drafts_prereleases_and_other_platforms() {
        let mut draft = release("1.11.0", &["runtime-x86_64"]);
        draft["draft"] = true.into();
        let mut beta = release("1.10.0", &["runtime-x86_64"]);
        beta["prerelease"] = true.into();
        let pin = Pin {
            version: "1.8.0",
            ..RUNTIME
        };
        let text = answer(&[
            release("continuous", &["runtime-x86_64"]),
            release("1.9.0", &["runtime-x86_64"]),
            release("1.9.1", &["runtime-x86_64"]),
            release("1.9.2", &["other"]),
            draft,
            beta,
            release("old", &["runtime-x86_64"]),
        ]);
        assert_eq!(
            newer_release(&text, &pin),
            Ok(Some(Newer {
                version: "1.9.1".into(),
                page: "https://github.com/o/r/releases/tag/1.9.1".into(),
                file: "runtime-x86_64".into(),
                digest: Some("1.9.1digest".into()),
            }))
        );
        // A newer release without this platform's archive (libvosk 0.3.45 has no macOS build)
        // leaves the pin current.
        let vosk = answer(&[
            release("v0.3.42", &["vosk-osx-0.3.42.zip"]),
            release("v0.3.45", &["vosk-linux-x86_64-0.3.45.zip"]),
        ]);
        assert_eq!(newer_release(&vosk, &VOSK), Ok(None));
        let vosk = answer(&[release("v0.3.50", &["vosk-osx-0.3.50.zip"])]);
        assert_eq!(
            newer_release(&vosk, &VOSK).unwrap().map(|newer| newer.file),
            Some("vosk-osx-0.3.50.zip".into())
        );
        assert_eq!(
            newer_release(
                &answer(&[release("20251108", &["runtime-x86_64"])]),
                &RUNTIME
            ),
            Ok(None)
        );
    }

    #[test]
    fn an_unreadable_answer_is_an_error() {
        for text in [
            "<html>504 Gateway Time-out</html>",
            r#"{"message": "API rate limit exceeded"}"#,
            "[]",
            &answer(&[release("continuous", &[])]),
        ] {
            let error = newer_release(text, &RUNTIME).unwrap_err();
            assert!(
                error.starts_with("GitHub's list of releases is unreadable"),
                "{error}"
            );
        }
        let moving = Pin {
            version: "continuous",
            ..RUNTIME
        };
        assert!(newer_release(&answer(&[release("1.0", &[])]), &moving).is_err());
        assert!(newer_commit("{}", &ENGINE).is_err());
    }

    #[test]
    fn a_commit_pin_is_behind_when_the_branch_moved() {
        let head = |sha: &str| format!(r#"{{"sha":"{sha}","html_url":"https://x/{sha}"}}"#);
        assert_eq!(newer_commit(&head("abc"), &ENGINE), Ok(None));
        assert_eq!(
            newer_commit(&head("def"), &ENGINE)
                .unwrap()
                .map(|newer| newer.version),
            Some("def".into())
        );
    }

    #[test]
    fn the_check_reports_each_pin_that_is_behind_asking_once_per_repository() {
        let pins = [
            (RUNTIME, "pinned.rs"),
            (VOSK, "pinned.rs"),
            (
                Pin {
                    file: "vosk-win64-0.3.42.zip",
                    ..VOSK
                },
                "pinned.rs",
            ),
            (ENGINE, "shipped.rs"),
        ];
        let asked = std::cell::RefCell::new(Vec::new());
        let github = |runtime: &'static str, vosk: &'static str, engine: &'static str| {
            let asked = &asked;
            move |path: &str| -> Result<String> {
                asked.borrow_mut().push(path.to_string());
                Ok(match path {
                    "repos/o/runtime/releases?per_page=100" => {
                        answer(&[release(runtime, &["runtime-x86_64"])])
                    }
                    "repos/o/vosk/releases?per_page=100" => answer(&[release(
                        vosk,
                        &[&file_in(&VOSK, vosk), "vosk-win64-0.3.42.zip"],
                    )]),
                    "repos/o/engine/commits/master" => format!(r#"{{"sha":"{engine}"}}"#),
                    other => panic!("{other}"),
                })
            }
        };
        assert_eq!(check(&pins, github("20251108", "v0.3.42", "abc")), Ok(None));
        assert_eq!(asked.borrow().len(), 3, "{:?}", asked.borrow());

        let text = check(&pins, github("99991231", "v0.3.42", "def"))
            .unwrap()
            .expect("the runtime and the engine are behind");
        assert!(
            text.contains(
                "- **the runtime** (`o/runtime`, `runtime-x86_64`): pinned `20251108`, newest \
             [`99991231`](https://github.com/o/r/releases/tag/99991231).\n  Its \
             `runtime-x86_64` has SHA-256 `99991231digest`.\n  Change the pin in `pinned.rs`.\n"
            ),
            "{text}"
        );
        assert!(text.contains(
            "- **the engine** (`o/engine`, `engine.tar.gz`): pinned `abc`, newest [`def`]"
        ));
        assert!(text.contains("Change the pin in `shipped.rs`."));
        assert!(!text.contains("libvosk"), "{text}");

        // A newer release with the macOS archive but not the Windows one: only macOS is behind.
        let text = check(&pins, github("20251108", "v0.3.50", "abc"))
            .unwrap()
            .unwrap();
        assert!(text.contains("`vosk-osx-0.3.42.zip`): pinned `v0.3.42`, newest [`v0.3.50`]"));
        assert!(!text.contains("vosk-win64"), "{text}");
    }

    #[test]
    fn a_failed_or_unreadable_lookup_fails_the_check() {
        let pins = [(RUNTIME, "pinned.rs")];
        let down = |_: &str| -> Result<String> { Err("HTTP 504".into()) };
        assert_eq!(check(&pins, down), Err("HTTP 504".into()));
        let garbled = |_: &str| -> Result<String> { Ok("{}".into()) };
        let error = check(&pins, garbled).unwrap_err();
        assert!(
            error.starts_with("the runtime (o/runtime): GitHub's"),
            "{error}"
        );
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
