//! The architecture boundaries of the frontends' shipped code, checked as a test (CI runs it in
//! `check` and in every `gpui` job; `cargo nextest run -p xtask boundaries` runs it alone).
//!
//! The app and the CLI talk to the core by `CoreApi` method name, and read its errors and log
//! levels from `kchess-contracts`. Both can also embed a core, so they depend on `kchess-core`,
//! and the compiler alone cannot keep the rest of their code from reaching into it. This test
//! does: only the files that start, configure or host the core name `kchess_core`. Tests (the
//! app's `e2e`, `tests.rs` files and inline test modules) may name anything.

use std::path::Path;

use crate::util::{Result, root};

/// What only some files of the frontends may name.
struct Rule {
    /// The text that marks a use.
    needle: &'static str,
    /// Why, as the failure says it.
    why: &'static str,
    /// The files, from the repository root, that may name it.
    allowed: &'static [&'static str],
}

/// The frontends' sources, searched from the repository root.
const FRONTENDS: [&str; 2] = ["apps/gpui/src", "apps/cli/src"];

const RULES: [Rule; 2] = [
    Rule {
        needle: "kchess_core",
        why: "frontends call the core by `CoreApi` method name and read `CoreError` and `Level` from \
          `kchess_contracts`; only the files that start, configure or host the core name \
          `kchess_core` (tooling/xtask/src/boundaries.rs)",
        allowed: &[
            // Starting the core (embedded or through the service) and answering its host requests.
            "apps/gpui/src/core.rs",
            "apps/cli/src/core.rs",
            // The programs and switches the core is configured with.
            "apps/gpui/src/profile.rs",
            // Log lines are redacted as the core's are.
            "apps/gpui/src/log.rs",
            // The App Store build: no updater, and no Stockfish download in the engine settings.
            "apps/gpui/src/updates.rs",
            "apps/gpui/src/pages/settings/engine.rs",
            // Voice input's test audio, behind the core's test switch.
            "apps/gpui/src/voice/capture.rs",
        ],
    },
    Rule {
        needle: "reqwest",
        why: "the core owns networking: a frontend asks it by `CoreApi` method, and only the \
          updater and Lichess's pictures make their own requests (tooling/xtask/src/boundaries.rs)",
        allowed: &[
            // The update feed and installers: the app replaces itself, which a core it may be
            // sharing with other windows cannot do for it. No login is sent.
            "apps/gpui/src/updates.rs",
            // Pictures Lichess serves (broadcast covers), from its image hosts only, drawn by the
            // app straight from the bytes. No login is sent.
            "apps/gpui/src/remote_image.rs",
        ],
    },
];

/// Whether `path` holds only tests, which may name anything.
fn is_test_file(path: &str) -> bool {
    path.contains("/e2e/") || path.contains("/tests/") || path.ends_with("/tests.rs")
}

/// The shipped part of a source file: what comes before its inline test module.
fn shipped(text: &str) -> &str {
    text.find("#[cfg(test)]\nmod tests")
        .map_or(text, |at| &text[..at])
}

/// Each use of a rule's needle outside its allowed files, as `path:line: why`.
fn offences(rule: &Rule, files: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (path, text) in files {
        if is_test_file(path) || rule.allowed.contains(&path.as_str()) {
            continue;
        }
        for (index, line) in shipped(text).lines().enumerate() {
            if line.contains(rule.needle) {
                found.push(format!(
                    "{path}:{}: names `{}`: {}",
                    index + 1,
                    rule.needle,
                    rule.why
                ));
            }
        }
    }
    found
}

/// The Rust sources under `dir` of `repo`, as (path from `repo`, text), in path order.
fn sources_under(repo: &Path, dir: &str, into: &mut Vec<(String, String)>) -> Result {
    let full = repo.join(dir);
    let entries =
        std::fs::read_dir(&full).map_err(|cause| format!("{}: {cause}", full.display()))?;
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        let path = format!("{dir}/{name}");
        let full = repo.join(&path);
        if full.is_dir() {
            sources_under(repo, &path, into)?;
        } else if name.ends_with(".rs") {
            let text = std::fs::read_to_string(&full)
                .map_err(|cause| format!("{}: {cause}", full.display()))?;
            into.push((path, text));
        }
    }
    Ok(())
}

/// Every rule's offences in `repo`'s frontends.
fn check_in(repo: &Path) -> Result {
    let mut files = Vec::new();
    for dir in FRONTENDS {
        sources_under(repo, dir, &mut files)?;
    }
    let found: Vec<String> = RULES
        .iter()
        .flat_map(|rule| offences(rule, &files))
        .collect();
    if found.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} use(s) across an architecture boundary:\n{}",
            found.len(),
            found.join("\n")
        ))
    }
}

/// The files that call the core by a method's name as text: its plumbing, which sends what a
/// typed call (`kchess_contracts::calls`) names.
const NAMED_CALLS: [&str; 2] = ["apps/gpui/src/core.rs", "apps/cli/src/core.rs"];

/// Each shipped frontend call of a core method by its name as text (`.call("library", …)`),
/// whatever its line breaks, as `path: method`.
fn named_calls(files: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (path, text) in files {
        if is_test_file(path) || NAMED_CALLS.contains(&path.as_str()) {
            continue;
        }
        let packed: String = shipped(text)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        for call in [".call(\"", ".call_now(\""] {
            for (at, _) in packed.match_indices(call) {
                let rest = &packed[at + call.len()..];
                let method = rest.split('"').next().unwrap_or_default();
                found.push(format!(
                    "{path}: calls `{method}` by name; declare it in `calls!` \
                     (crates/kchess-contracts/src/calls.rs) and send it with `request`"
                ));
            }
        }
    }
    found
}

const RULE: Rule = Rule {
    needle: "kchess_core",
    why: "a reason",
    allowed: &["apps/gpui/src/core.rs"],
};

fn file(path: &str, text: &str) -> (String, String) {
    (path.to_string(), text.to_string())
}

#[test]
fn the_frontends_call_the_core_with_typed_calls() {
    let mut files = Vec::new();
    for dir in FRONTENDS {
        sources_under(&root(), dir, &mut files).expect("the frontends' sources");
    }
    let found = named_calls(&files);
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn a_call_by_name_is_found_across_lines() {
    let files = [
        file(
            "apps/gpui/src/core.rs",
            "core.call(\"close\", Vec::new())\n",
        ),
        file(
            "apps/gpui/src/page.rs",
            "let a = core.request(calls::Library {});\nlet b = core\n    .call(\n        \"library\",\n        Vec::new(),\n    );\nbridge.call_now(\"usage\", vec![]);\n",
        ),
        file(
            "apps/gpui/src/e2e/page.rs",
            "core.call(\"library\", Vec::new())\n",
        ),
    ];
    let found = named_calls(&files);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].starts_with("apps/gpui/src/page.rs: calls `library` by name"));
    assert!(found[1].starts_with("apps/gpui/src/page.rs: calls `usage` by name"));
}

#[test]
fn the_frontends_keep_their_boundaries() {
    if let Err(failure) = check_in(&root()) {
        panic!("{failure}");
    }
}

#[test]
fn a_use_outside_the_allowed_files_is_named_with_its_line() {
    let files = [
        file("apps/gpui/src/core.rs", "use kchess_core::Core;\n"),
        file(
            "apps/gpui/src/page.rs",
            "use x;\nlet a = kchess_core::app_store();\n",
        ),
    ];
    assert_eq!(
        offences(&RULE, &files),
        ["apps/gpui/src/page.rs:2: names `kchess_core`: a reason"]
    );
}

#[test]
fn the_contracts_types_are_read_from_the_contracts() {
    let files = [file(
        "apps/gpui/src/page.rs",
        "use kchess_contracts::Level;\nlog(kchess_core::Level::Warn);\n",
    )];
    assert_eq!(
        offences(&RULE, &files),
        ["apps/gpui/src/page.rs:2: names `kchess_core`: a reason"]
    );
}

#[test]
fn tests_may_name_anything() {
    let files = [
        file("apps/gpui/src/e2e/puzzles.rs", "kchess_core::puzzles\n"),
        file("apps/gpui/src/updates/tests.rs", "kchess_core::loopback\n"),
        file("apps/cli/src/tests/play.rs", "kchess_core::env\n"),
        file(
            "apps/gpui/src/page.rs",
            "fn shown() {}\n#[cfg(test)]\nmod tests {\n    use kchess_core::env;\n}\n",
        ),
    ];
    assert!(offences(&RULE, &files).is_empty());
    assert!(!is_test_file("apps/gpui/src/e2e.rs"));
    assert!(!is_test_file("apps/gpui/src/latests.rs"));
}

#[test]
fn only_code_before_the_test_module_ships() {
    assert_eq!(shipped("a\n#[cfg(test)]\nmod tests {}\n"), "a\n");
    let helper = "a\n#[cfg(test)]\nfn helper() {}\n";
    assert_eq!(shipped(helper), helper);
}

#[test]
fn a_repository_that_crosses_a_boundary_fails() {
    let repo = std::env::temp_dir().join(format!("kchess-boundaries-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&repo);
    for dir in FRONTENDS {
        std::fs::create_dir_all(repo.join(dir).join("pages")).unwrap();
    }
    std::fs::write(
        repo.join("apps/gpui/src/core.rs"),
        "use kchess_core::Core;\n",
    )
    .unwrap();
    std::fs::write(repo.join("apps/gpui/src/notes.txt"), "kchess_core\n").unwrap();
    assert_eq!(check_in(&repo), Ok(()));

    std::fs::write(
        repo.join("apps/gpui/src/pages/home.rs"),
        "kchess_core::env::var(\"X\");\n",
    )
    .unwrap();
    let failure = check_in(&repo).unwrap_err();
    assert!(
        failure.starts_with("1 use(s) across an architecture boundary"),
        "{failure}"
    );
    assert!(
        failure.contains("apps/gpui/src/pages/home.rs:1: names `kchess_core`"),
        "{failure}"
    );
    std::fs::remove_dir_all(&repo).unwrap();

    let missing =
        std::env::temp_dir().join(format!("kchess-boundaries-none-{}", std::process::id()));
    assert!(check_in(&missing).is_err());
}

/// Each code block in a doc comment of `files`, as `path:line`: a doctest, which `check` does not
/// run.
fn doc_code_blocks(files: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (path, text) in files {
        for (index, line) in text.lines().enumerate() {
            let line = line.trim_start();
            let doc = line
                .strip_prefix("///")
                .or_else(|| line.strip_prefix("//!"));
            if doc.is_some_and(|doc| doc.trim_start().starts_with("```")) {
                found.push(format!("{path}:{}", index + 1));
            }
        }
    }
    found
}

/// The workspace has no doctests, so `check` runs none: `cargo test --doc` compiled every
/// library again under rustdoc for nothing (34 s of CI's check job, 2026-11-10). A code block in
/// a doc comment would be a doctest nobody runs; write the example as a `#[test]` instead.
#[test]
fn no_doc_comment_holds_a_doctest() {
    let repo = root();
    let mut files = Vec::new();
    for dir in ["apps", "crates", "tooling"] {
        sources_under(&repo, dir, &mut files).unwrap();
    }
    assert!(files.len() > 100, "only {} sources found", files.len());
    let found = doc_code_blocks(&files);
    assert!(
        found.is_empty(),
        "doctests that check does not run: {found:?}"
    );
}

#[test]
fn a_fenced_block_in_a_doc_comment_is_found() {
    let files = [
        file("a.rs", "/// Text.\n    /// ```\n/// x\n/// ```\n"),
        file("b.rs", "//! ```rust\n// ```\nlet s = \"```\";\n"),
    ];
    assert_eq!(doc_code_blocks(&files), ["a.rs:2", "a.rs:4", "b.rs:1"]);
}
