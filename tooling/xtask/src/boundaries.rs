//! The architecture boundaries of the frontends' shipped code, checked as a test (CI runs it in
//! `check` and in every `gpui` job; `cargo nextest run -p xtask boundaries` runs it alone).
//!
//! The app and the CLI talk to the core by `CoreApi` method name, and read its errors and log
//! levels from `kchess-contracts`. Both can also embed a core, so they depend on `kchess-core`,
//! and the compiler alone cannot keep the rest of their code from reaching into it. This test
//! does: only the files that start, configure or host the core name `kchess_core`, and only the
//! updater and the picture loader make HTTP requests of their own (`reqwest`). Tests (the app's
//! `e2e`, `tests.rs` files and inline test modules) may name anything.
//!
//! It also holds the app's rules of look that span every page (spinners, named selects, corners,
//! steady clocks), which no test of one page can see.

use std::path::Path;

use crate::members::APP_SOURCES;
use crate::util::{Result, root, sources_under};

/// What only some files of the frontends may name.
struct Rule {
    /// The text that marks a use.
    needle: &'static str,
    /// Why, as the failure says it.
    why: &'static str,
    /// The files, from the repository root, that may name it.
    allowed: &'static [&'static str],
}

/// The frontends' sources, searched from the repository root: the app's and the CLI's.
const FRONTENDS: [&str; 13] = [
    APP_SOURCES[0],
    APP_SOURCES[1],
    APP_SOURCES[2],
    APP_SOURCES[3],
    APP_SOURCES[4],
    APP_SOURCES[5],
    APP_SOURCES[6],
    APP_SOURCES[7],
    APP_SOURCES[8],
    APP_SOURCES[9],
    APP_SOURCES[10],
    APP_SOURCES[11],
    "apps/cli/src",
];

const RULES: [Rule; 2] = [
    Rule {
        needle: "kchess_core",
        why: "frontends call the core by `CoreApi` method name and read `CoreError` and `Level` from \
          `kchess_contracts`; only the files that start, configure or host the core name \
          `kchess_core` (tooling/xtask/src/boundaries.rs)",
        allowed: &[
            // Starting the core (embedded or through the service) and answering its host requests.
            // The command line starts it through `kchess-service` and names it nowhere.
            "apps/gpui/src/core.rs",
        ],
    },
    Rule {
        needle: "reqwest",
        why: "the core owns networking: a frontend asks it by `CoreApi` method, and only the \
          updater and Lichess's pictures make their own requests (tooling/xtask/src/boundaries.rs)",
        allowed: &[
            // The update feed and installers: the app replaces itself, which a core it may be
            // sharing with other windows cannot do for it. No login is sent.
            "apps/gpui/src/updates/download.rs",
            // Pictures Lichess serves (broadcast covers), from its image hosts only, drawn by the
            // app straight from the bytes. No login is sent.
            "crates/kchess-watch/src/remote_image.rs",
        ],
    },
];

/// Whether `path` holds only tests, which may name anything.
fn is_test_file(path: &str) -> bool {
    path.contains("/e2e/") || path.contains("/tests/") || path.ends_with("/tests.rs")
}

/// The shipped part of a source file: what comes before its inline test module, which is last
/// (`every_test_module_comes_after_the_code`).
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

/// The files that may also send a method whose name is only known as the program runs: the
/// command line's `call` command sends the method the person names.
const NAMED_AT_RUN_TIME: [&str; 1] = ["apps/cli/src/main.rs"];

/// Each shipped frontend call of a core method by its name (`.call("library", …)`, or a name in
/// a variable, `.call(method, …)`), whatever its line breaks, as `path: what`.
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
        for call in [".call(", ".call_now("] {
            for (at, _) in packed.match_indices(call) {
                let rest = &packed[at + call.len()..];
                let named = match rest.strip_prefix('"') {
                    Some(literal) => format!("`{}`", literal.split('"').next().unwrap_or_default()),
                    None if NAMED_AT_RUN_TIME.contains(&path.as_str()) => continue,
                    None => format!(
                        "the method named in `{}`",
                        rest.split([',', ')']).next().unwrap_or_default()
                    ),
                };
                found.push(format!(
                    "{path}: calls {named} by name; declare it in `core_api!` \
                     (crates/kchess-contracts/src/core_api.rs) and send its typed call with \
                     `request`"
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

/// A name in a variable escaped the check once (`call(if masters { "mastersGame" } else { … })`
/// and `call(method, args)`): it is found too, except in the command line's `call` command.
#[test]
fn a_call_by_a_name_in_a_variable_is_found() {
    let files = [
        file(
            "apps/gpui/src/page.rs",
            "let call = core.call(\n    method,\n    args,\n);\nbridge.call_now(name, vec![]);\n",
        ),
        file(
            "apps/cli/src/main.rs",
            "print(&core.call(method, parameters).await?);\ncore.call(\"usage\", vec![]);\n",
        ),
    ];
    let found = named_calls(&files);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(
        found[0].starts_with("apps/gpui/src/page.rs: calls the method named in `method` by name")
    );
    assert!(
        found[1].starts_with("apps/gpui/src/page.rs: calls the method named in `name` by name")
    );
    assert!(found[2].starts_with("apps/cli/src/main.rs: calls `usage` by name"));
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

/// Each item of `files` after a test module, as `path:line` of its first line: code after the
/// tests is code the boundary rules above do not read (`shipped`), and code a reader of the file
/// looks for above them. Further test modules may follow.
fn code_after_tests(files: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (path, text) in files {
        let lines: Vec<&str> = text.lines().collect();
        let (mut tested, mut index) = (false, 0);
        while let Some(line) = lines.get(index) {
            let test_module = *line == "#[cfg(test)]"
                && lines
                    .get(index + 1)
                    .is_some_and(|next| next.starts_with("mod ") && next.ends_with('{'));
            if test_module {
                tested = true;
                index = module_end(&lines, index + 2);
            } else if tested && starts_item(line) {
                found.push(format!("{path}:{}", index + 1));
                // Its other attributes and its first line are the same item.
                while lines.get(index).is_some_and(|line| line.starts_with("#[")) {
                    index += 1;
                }
            }
            index += 1;
        }
    }
    found
}

/// The line that closes a module whose body starts at `from`: the first `}` at the left edge
/// outside a raw string (tests hold whole sources in `r#"…"#`, closings and all).
fn module_end(lines: &[&str], from: usize) -> usize {
    let mut raw = false;
    for (index, line) in lines.iter().enumerate().skip(from) {
        if raw {
            raw = !line.contains("\"#");
        } else if *line == "}" {
            return index;
        } else if let Some(at) = line.find("r#\"") {
            raw = !line[at + 3..].contains("\"#");
        }
    }
    lines.len()
}

/// Whether `line` begins an item at the top of a file: code, not a comment, a blank or a closing.
fn starts_item(line: &str) -> bool {
    !line.is_empty() && !line.starts_with([' ', '\t', '}', ')', ']']) && !line.starts_with("//")
}

/// Tests come last in every file of the workspace, so `shipped` reads all of a file's code.
#[test]
fn every_test_module_comes_after_the_code() {
    let repo = root();
    let mut files = Vec::new();
    for dir in ["apps", "crates", "tooling"] {
        sources_under(&repo, dir, &mut files).unwrap();
    }
    assert!(files.len() > 100, "only {} sources found", files.len());
    let found = code_after_tests(&files);
    assert!(found.is_empty(), "code after a test module: {found:?}");
}

#[test]
fn code_after_a_test_module_is_found_with_its_line() {
    let text =
        "fn a() {}\n\n#[cfg(test)]\nmod tests {\n    fn b() {}\n}\n\n/// Shipped.\npub fn c() {}\n";
    assert_eq!(
        code_after_tests(&[file("crates/x/src/lib.rs", text)]),
        ["crates/x/src/lib.rs:9"]
    );
    let two = "fn a() {}\n\n#[cfg(test)]\nmod tests {\n}\n\n// The variants.\n#[cfg(test)]\nmod variant_tests {\n    fn b() {}\n}\n";
    assert!(code_after_tests(&[file("crates/x/src/lib.rs", two)]).is_empty());
    // A test's source text is not the file's: its closing brace does not end the module.
    let fixture = "#[cfg(test)]\nmod tests {\n    const TEXT: &str = r#\"\nfn x() {\n}\n\"#;\n}\n";
    assert!(code_after_tests(&[file("crates/x/src/lib.rs", fixture)]).is_empty());
    let attribute = "#[cfg(test)]\nmod tests {\n}\n#[derive(Debug)]\nstruct S;\n";
    assert_eq!(
        code_after_tests(&[file("crates/x/src/lib.rs", attribute)]),
        ["crates/x/src/lib.rs:4"]
    );
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

/* The crates the app is built from never build the core. */

/// The core's crates. A UI crate holds its core through `kchess-client`; one that built these
/// rebuilt after every edit to the core (2026-11-20: ten did, through `kchess-app` naming
/// `kchess-service`, which ran their builds and tests again for 91 of the core's 221 commits that
/// month that touched no frontend). Only their tests may, through `test-support`.
const CORE_CRATES: [&str; 2] = ["kchess-core", "kchess-service"];

/// Each UI crate that builds a core crate through the members it always depends on (not an
/// optional dependency, a test's, or a build script's), as the path it takes.
fn ui_crates_building_the_core(metadata: &serde_json::Value) -> Vec<String> {
    let packages = metadata["packages"].as_array().cloned().unwrap_or_default();
    let members: std::collections::HashSet<&str> = packages
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect();
    let built: std::collections::HashMap<&str, Vec<&str>> = packages
        .iter()
        .filter_map(|package| {
            let uses = package["dependencies"]
                .as_array()?
                .iter()
                .filter(|dependency| dependency["kind"].is_null() && dependency["optional"] != true)
                .filter_map(|dependency| dependency["name"].as_str())
                .filter(|name| members.contains(name))
                .collect();
            Some((package["name"].as_str()?, uses))
        })
        .collect();
    let mut found = Vec::new();
    let mut ui: Vec<&str> = built
        .keys()
        .copied()
        .filter(|name| crate::members::role(name) == crate::members::Role::Ui)
        .collect();
    ui.sort_unstable();
    for start in ui {
        let mut paths = vec![vec![start]];
        let mut seen = std::collections::HashSet::from([start]);
        while let Some(path) = paths.pop() {
            let Some(last) = path.last() else { continue };
            if CORE_CRATES.contains(last) {
                found.push(path.join(" → "));
                break;
            }
            for next in built.get(last).into_iter().flatten() {
                if seen.insert(next) {
                    let mut longer = path.clone();
                    longer.push(next);
                    paths.push(longer);
                }
            }
        }
    }
    found
}

#[test]
fn the_crates_the_app_is_built_from_never_build_the_core() {
    let text = crate::util::output(
        "cargo",
        &["metadata", "--no-deps", "--format-version", "1", "--locked"],
    )
    .expect("cargo metadata");
    let metadata = serde_json::from_str(&text).expect("its JSON");
    let found = ui_crates_building_the_core(&metadata);
    assert!(
        found.is_empty(),
        "a UI crate builds the core; name `kchess-client` instead: {found:?}"
    );
}

#[test]
fn a_ui_crate_reaching_the_core_is_named_with_its_path() {
    let package = |name: &str, dependencies: serde_json::Value| serde_json::json!({ "name": name, "dependencies": dependencies });
    let normal = |name: &str| serde_json::json!({ "name": name, "kind": null, "optional": false });
    let metadata = serde_json::json!({ "packages": [
        package("kchess-board", serde_json::json!([normal("kchess-app")])),
        package("kchess-app", serde_json::json!([
            normal("kchess-client"),
            // Optional, and a test's: neither is built with the crate.
            { "name": "kchess-core", "kind": null, "optional": true },
            { "name": "kchess-service", "kind": "dev", "optional": false },
        ])),
        package("kchess-client", serde_json::json!([])),
        package("kchess-core", serde_json::json!([])),
        package("kchess-service", serde_json::json!([normal("kchess-core")])),
    ]});
    assert!(ui_crates_building_the_core(&metadata).is_empty());

    let metadata = serde_json::json!({ "packages": [
        package("kchess-board", serde_json::json!([normal("kchess-app")])),
        package("kchess-app", serde_json::json!([normal("kchess-service")])),
        package("kchess-service", serde_json::json!([normal("kchess-core")])),
        package("kchess-core", serde_json::json!([])),
    ]});
    assert_eq!(
        ui_crates_building_the_core(&metadata),
        [
            "kchess-app → kchess-service",
            "kchess-board → kchess-app → kchess-service"
        ]
    );
}

/* The app's look: rules that hold across every page, which no test of one page can see. */

/// The app's shipped sources, as (path from the repository root, text before its test module).
fn app_sources() -> Vec<(String, String)> {
    let mut files = Vec::new();
    for dir in APP_SOURCES {
        sources_under(&root(), dir, &mut files).expect("the app's sources");
    }
    files
        .into_iter()
        .filter(|(path, _)| !is_test_file(path))
        .map(|(path, text)| {
            let text = shipped(&text).replace("\r\n", "\n");
            (path, text)
        })
        .collect()
}

/// Each item of a UI crate the app is built from compiled only for that crate's own tests, as
/// `path:line`: the app's tests build the crate without `cfg(test)`, so such an item is missing
/// from them. A test module is the only test-only item; anything the app's tests use sits behind
/// the crate's `test-support` feature as well. History's `search_lichess`, test-only on macOS,
/// failed only the macOS build (2026-11-19), and a `cfg(test)` check once went quiet unseen.
fn test_only_items(files: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (path, text) in files {
        if path.starts_with("apps/") {
            continue;
        }
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let line = line.trim();
            let test_only = line.starts_with("#[cfg(")
                && line.contains("test")
                && !line.contains("feature = \"test-support\"");
            let item = lines[index + 1..]
                .iter()
                .map(|next| next.trim())
                .find(|next| !next.starts_with("#["));
            let module =
                item.is_some_and(|item| item.starts_with("mod ") || item.starts_with("pub mod "));
            if test_only && !module {
                found.push(format!("{path}:{}", index + 1));
            }
        }
    }
    found
}

#[test]
fn the_app_crates_compile_for_the_apps_tests_as_for_their_own() {
    let mut files = Vec::new();
    for dir in &APP_SOURCES[1..] {
        sources_under(&root(), dir, &mut files).expect("the app's crates' sources");
    }
    let found = test_only_items(&files);
    assert!(
        found.is_empty(),
        "test-only items outside test-support: {found:?}"
    );
}

#[test]
fn a_test_only_item_is_found_but_not_a_test_module() {
    let files = [
        file(
            "crates/kchess-ui/src/a.rs",
            "#[cfg(test)]\nmod tests {}\n#[cfg(all(test, target_os = \"macos\"))]\n\
             #[allow(unused)]\npub fn shown() {}\n#[cfg(any(test, feature = \"test-support\"))]\n\
             pub fn kept() {}\n",
        ),
        file("apps/gpui/src/a.rs", "#[cfg(test)]\nfn helper() {}\n"),
    ];
    assert_eq!(test_only_items(&files), ["crates/kchess-ui/src/a.rs:3"]);
}

/// Each file with a still loader: the loader icon outside `spinner`, or a "Loading…" written as
/// a plain hint, which has no spinner (`loading` has one).
fn still_loaders(files: &[(String, String)]) -> Vec<String> {
    files
        .iter()
        .filter(|(path, text)| {
            let allowed = usize::from(path == "crates/kchess-ui/src/widgets/status.rs");
            text.matches("icon(\"loader-circle\")").count() > allowed
                || text.contains("hint(\"Loading")
        })
        .map(|(path, _)| path.clone())
        .collect()
}

/// Each `Select` built without an accessible name in the builder chain that follows it, as
/// `path:line`.
fn unnamed_selects(files: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (path, text) in files {
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains("Select::new(") || line.trim_start().starts_with("//") {
                continue;
            }
            let chain = lines[index..lines.len().min(index + 8)].join("\n");
            if !chain.contains("accessibility_label") {
                found.push(format!("{path}:{}", index + 1));
            }
        }
    }
    found
}

/// Each corner rounded by a literal size rather than the `radii` scale, as `path:line`; only
/// Settings' theme preview, a scale model of a page, rounds its own.
fn stray_corners(files: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (path, text) in files {
        let preview = text
            .find("fn theme_preview(")
            .map(|start| (start, start + text[start..].find("\n}").unwrap_or(0)));
        for (at, _) in text.match_indices(".rounded") {
            let call = &text[at..];
            let literal = call.find('(').is_some_and(|open| {
                call[open + 1..].starts_with("px(") && !call[..open].contains('\n')
            });
            let in_preview = preview.is_some_and(|(start, end)| (start..end).contains(&at));
            if literal && !in_preview {
                found.push(format!("{path}:{}", text[..at].lines().count()));
            }
        }
    }
    found
}

/// Each file drawing a large ticking time without tabular figures, whose digits then shift
/// sideways on every tick. The clocks of Puzzle Rush, timed practice runs and Over the board's
/// chess clock draw through `widgets::timer`; the title bar's "Updating N%" is anchored on the
/// right, so a proportional figure moved the button's left edge with every percent.
fn proportional_timers(files: &[(String, String)]) -> Vec<String> {
    let text_of = |wanted: &str| {
        files
            .iter()
            .find(|(path, _)| path == wanted)
            .map_or("", |(_, text)| text.as_str())
    };
    let mut found: Vec<String> = [
        "crates/kchess-puzzles/src/rush.rs",
        "crates/kchess-practice/src/run.rs",
        "crates/kchess-local/src/clock.rs",
    ]
    .iter()
    .filter(|path| !text_of(path).contains("widgets::timer("))
    .map(|path| path.to_string())
    .collect();
    let updates = "apps/gpui/src/pages/settings/updates.rs";
    let text = text_of(updates);
    let button = text
        .find("fn title_bar_button")
        .map(|at| &text[at..])
        .map(|rest| &rest[..rest.find("\n}\n").unwrap_or(rest.len())]);
    if !button.is_some_and(|button| button.contains(".font_features(crate::widgets::tabular())")) {
        found.push(updates.to_string());
    }
    found
}

/// Every wait turns: the Lichess seek card once showed a loader that never moved.
#[test]
fn every_loader_in_the_app_is_the_turning_spinner() {
    let found = still_loaders(&app_sources());
    assert!(found.is_empty(), "a still loader in {found:?}");
}

#[test]
fn a_still_loader_is_found() {
    let files = [
        file(
            "crates/kchess-ui/src/widgets/status.rs",
            "icon(\"loader-circle\")",
        ),
        file("apps/gpui/src/a.rs", "hint(\"Loading games…\")"),
        file("apps/gpui/src/b.rs", "icon(\"loader-circle\")"),
        file("apps/gpui/src/c.rs", "widgets::loading(\"Loading games…\")"),
    ];
    assert_eq!(
        still_loaders(&files),
        ["apps/gpui/src/a.rs", "apps/gpui/src/b.rs"]
    );
}

/// A select's placeholder and value are never its accessible name (GPUI Kit 0.9), so each one
/// is named where it is built. Most of the unnamed ones sat behind a connected account, where
/// the end-to-end audit cannot reach.
#[test]
fn every_select_in_the_app_is_named() {
    let found = unnamed_selects(&app_sources());
    assert!(found.is_empty(), "an unnamed select at {found:?}");
}

#[test]
fn an_unnamed_select_is_found_with_its_line() {
    let files = [file(
        "apps/gpui/src/a.rs",
        "Select::new(&a)\n    .accessibility_label(\"Speed\");\n// Select::new(&b)\n\
         Select::new(&c)\n    .w_full();\n",
    )];
    assert_eq!(unnamed_selects(&files), ["apps/gpui/src/a.rs:4"]);
}

#[test]
fn every_corner_in_the_app_comes_from_the_radius_scale() {
    let found = stray_corners(&app_sources());
    assert!(found.is_empty(), "corners outside `radii`: {found:?}");
}

#[test]
fn a_literal_corner_is_found_outside_the_theme_preview() {
    let files = [file(
        "apps/gpui/src/a.rs",
        "div().rounded(radii::BOX)\ndiv().rounded(px(3.))\nfn theme_preview() {\n    \
         div().rounded(px(2.))\n}\n",
    )];
    assert_eq!(stray_corners(&files), ["apps/gpui/src/a.rs:2"]);
}

#[test]
fn ticking_times_in_the_app_hold_still() {
    let found = proportional_timers(&app_sources());
    assert!(found.is_empty(), "proportional figures in {found:?}");
}

#[test]
fn a_clock_without_tabular_figures_is_found() {
    let files = [
        file("crates/kchess-puzzles/src/rush.rs", "widgets::timer(text)"),
        file("crates/kchess-practice/src/run.rs", "div().child(text)"),
        file("crates/kchess-local/src/clock.rs", "widgets::timer(text)"),
        file(
            "apps/gpui/src/pages/settings/updates.rs",
            "fn title_bar_button() {\n    label(percent)\n}\n",
        ),
    ];
    assert_eq!(
        proportional_timers(&files),
        [
            "crates/kchess-practice/src/run.rs",
            "apps/gpui/src/pages/settings/updates.rs"
        ]
    );
}

/// Each place reading the player's look and feel through the Settings model by hand, as
/// `path:line`: `settings::frontend(cx)` reads it in one call, and 37 hand-written five-line
/// chains of `global::<SettingsHandle>().0.read(cx).settings.frontend` had grown beside it.
fn frontend_read_by_hand(files: &[(String, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (path, text) in files {
        if path == "crates/kchess-app/src/settings.rs" {
            continue;
        }
        for (at, _) in text.match_indices("global::<SettingsHandle>()") {
            let chain: String = text[at..]
                .chars()
                .take(160)
                .filter(|c| !c.is_whitespace())
                .collect();
            let Some(rest) = chain.strip_prefix("global::<SettingsHandle>().0.read(") else {
                continue;
            };
            if rest
                .split_once(')')
                .is_some_and(|(_, after)| after.starts_with(".settings.frontend"))
            {
                found.push(format!("{path}:{}", text[..at].matches('\n').count() + 1));
            }
        }
    }
    found
}

#[test]
fn the_app_reads_its_look_and_feel_in_one_call() {
    let found = frontend_read_by_hand(&app_sources());
    assert!(
        found.is_empty(),
        "read with `crate::settings::frontend(cx)` at {found:?}"
    );
}

#[test]
fn a_look_and_feel_read_by_hand_is_found_with_its_line() {
    let files = [
        file(
            "apps/gpui/src/a.rs",
            "let sound = cx.global::<SettingsHandle>().0.read(cx).settings.sound;\nlet blindfold = cx\n    \
             .global::<SettingsHandle>()\n    .0\n    .read(cx)\n    .settings\n    .frontend\n    .blindfold;\n",
        ),
        file(
            "crates/kchess-app/src/settings.rs",
            "&cx.global::<SettingsHandle>().0.read(cx).settings.frontend",
        ),
    ];
    assert_eq!(frontend_read_by_hand(&files), ["apps/gpui/src/a.rs:3"]);
}
