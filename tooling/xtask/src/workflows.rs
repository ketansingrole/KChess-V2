//! Checks of the workflows and the repository's own actions (`.github/`): every bash step
//! parses and every file is valid YAML. `check-fast` runs them for a change to one
//! (`check::scope_of`).

/// Each workflow step's shell script that runs on bash, with its line: every step of a
/// workflow that never runs on Windows, and elsewhere those that say `shell: bash` (a Windows
/// runner's default shell is PowerShell).
fn bash_steps(text: &str) -> Vec<(usize, String)> {
    let windows = text.contains("windows");
    let lines: Vec<&str> = text.lines().collect();
    let indent = |line: &str| line.len() - line.trim_start().len();
    let mut found = Vec::new();
    let mut start = 0;
    while start < lines.len() {
        let line = lines[start];
        if !line.trim_start().starts_with("- ") {
            start += 1;
            continue;
        }
        // The step runs to the next item at its indentation or less.
        let item = indent(line);
        let end = (start + 1..lines.len())
            .find(|&n| {
                let next = lines[n];
                !next.trim().is_empty()
                    && (indent(next) < item
                        || (indent(next) == item && next.trim_start().starts_with("- ")))
            })
            .unwrap_or(lines.len());
        let step = &lines[start..end];
        let key = |name: &str| {
            step.iter().position(|line| {
                let trimmed = line.trim_start().trim_start_matches("- ");
                trimmed.starts_with(&format!("{name}:"))
            })
        };
        let shell = key("shell").map(|n| step[n].split(':').nth(1).unwrap_or("").trim());
        let runs_on_bash = match shell {
            Some(shell) => shell == "bash",
            None => !windows,
        };
        if let Some(run) = key("run")
            && runs_on_bash
        {
            let head = step[run]
                .split_once("run:")
                .map(|(_, rest)| rest.trim())
                .unwrap_or("");
            let script = if head == "|" || head == ">" {
                let base = indent(step[run]);
                step[run + 1..]
                    .iter()
                    .take_while(|line| line.trim().is_empty() || indent(line) > base)
                    .map(|line| line.get(base + 2..).unwrap_or("").to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                head.to_string()
            };
            found.push((start + run + 1, script));
        }
        start += 1;
    }
    found
}

/// The workflows and the repository's own actions (`.github/actions/<name>/action.yml`).
fn workflow_files() -> Vec<std::path::PathBuf> {
    let github = crate::util::root().join(".github");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(github.join("workflows"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "yml"))
        .collect();
    files.extend(
        std::fs::read_dir(github.join("actions"))
            .unwrap()
            .flatten()
            .map(|entry| entry.path().join("action.yml"))
            .filter(|path| path.is_file()),
    );
    files
}

/// 2026-11-07: a heredoc inside `$(...)` with a backtick in it parsed in zsh but not in bash,
/// and the TestFlight step failed on the first release that ran it. Every bash step parses.
#[cfg(unix)]
#[test]
fn every_workflow_step_parses_in_bash() {
    let mut checked = 0;
    for path in workflow_files() {
        let text = std::fs::read_to_string(&path).unwrap();
        for (line, script) in bash_steps(&text) {
            let mut bash = std::process::Command::new("bash")
                .arg("-n")
                .stdin(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            use std::io::Write as _;
            bash.stdin
                .take()
                .unwrap()
                .write_all(script.as_bytes())
                .unwrap();
            let out = bash.wait_with_output().unwrap();
            assert!(
                out.status.success(),
                "{}:{line} does not parse in bash: {}",
                path.display(),
                String::from_utf8_lossy(&out.stderr)
            );
            checked += 1;
        }
    }
    assert!(checked > 20, "only {checked} steps found");
}

/// 2026-11-09: a plain `run:` value with `: ` in it made release.yml invalid YAML, so GitHub
/// started no run for the v2026.11.6 tag; `bash_steps` reads lines and never noticed. Every
/// workflow parses as one YAML document.
#[test]
fn every_workflow_is_valid_yaml() {
    let mut checked = 0;
    for path in workflow_files() {
        let text = std::fs::read_to_string(&path).unwrap();
        let docs = yaml_rust2::YamlLoader::load_from_str(&text)
            .unwrap_or_else(|error| panic!("{} is not valid YAML: {error}", path.display()));
        assert_eq!(docs.len(), 1, "{} is not one YAML document", path.display());
        checked += 1;
    }
    assert!(checked > 5, "only {checked} workflows found");
}

/// Sections that hold a mapping or a list wherever they appear, which GitHub refuses when they
/// are empty: a key with nothing under it is YAML's null.
const NOT_EMPTY: &[&str] = &[
    "env",
    "with",
    "jobs",
    "steps",
    "permissions",
    "defaults",
    "outputs",
    "inputs",
    "secrets",
    "strategy",
    "matrix",
    "services",
    "concurrency",
    "runs",
];

/// The sections of `yaml` left empty, by their path from the document's root.
fn empty_sections(yaml: &yaml_rust2::Yaml, path: &str) -> Vec<String> {
    use yaml_rust2::Yaml;
    match yaml {
        Yaml::Hash(map) => map
            .iter()
            .flat_map(|(key, value)| {
                let key = key.as_str().unwrap_or_default();
                let here = format!("{path}.{key}");
                let empty = matches!(value, Yaml::Null) && NOT_EMPTY.contains(&key);
                empty
                    .then(|| here.clone())
                    .into_iter()
                    .chain(empty_sections(value, &here))
            })
            .collect(),
        Yaml::Array(items) => items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| empty_sections(item, &format!("{path}[{index}]")))
            .collect(),
        _ => Vec::new(),
    }
}

/// `env:` left with nothing under it, once its one variable moved to a shared action, made
/// release.yml a workflow GitHub refuses: every push failed it, and a tag would have started no
/// release. YAML reads it as null, so only a look at what each section holds sees it.
#[test]
fn no_workflow_leaves_a_section_empty() {
    let parse = |text: &str| {
        yaml_rust2::YamlLoader::load_from_str(text)
            .unwrap()
            .remove(0)
    };
    let broken = parse(
        "on:
  workflow_dispatch:
env:
jobs:
  a:
    steps:
      - uses: x
        with:
",
    );
    assert_eq!(
        empty_sections(&broken, ""),
        [".env", ".jobs.a.steps[0].with"],
        "an empty `workflow_dispatch:` is how GitHub spells a trigger without inputs"
    );
    for path in workflow_files() {
        let workflow = parse(&std::fs::read_to_string(&path).unwrap());
        let empty = empty_sections(&workflow, "");
        assert!(
            empty.is_empty(),
            "{} leaves {empty:?} empty",
            path.display()
        );
    }
}

/// The `rust-cache` steps of `yaml`, by their path from the root.
/// The repository's own Rust cache (`.github/actions/rust-cache`), the one user of rust-cache.
const RUST_CACHE_ACTION: &str = "./.github/actions/rust-cache";

fn rust_caches<'a>(yaml: &'a yaml_rust2::Yaml, path: &str) -> Vec<(String, &'a yaml_rust2::Yaml)> {
    use yaml_rust2::Yaml;
    match yaml {
        Yaml::Hash(map) => {
            let mut found = Vec::new();
            if yaml["uses"].as_str().is_some_and(|uses| {
                uses.starts_with("Swatinem/rust-cache@") || uses == RUST_CACHE_ACTION
            }) {
                found.push((path.to_string(), yaml));
            }
            found.extend(map.iter().flat_map(|(key, value)| {
                rust_caches(
                    value,
                    &format!("{path}.{}", key.as_str().unwrap_or_default()),
                )
            }));
            found
        }
        Yaml::Array(items) => items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| rust_caches(item, &format!("{path}[{index}]")))
            .collect(),
        _ => Vec::new(),
    }
}

/// The `rust-cache` steps of `yaml` that may save a cache from a pull request: those with no
/// `save-if`, or one that does not name main (`false` saves nothing; the action passes its own
/// `save-if` on, whose callers are checked). By their path from the root.
fn caches_saved_from_any_branch(yaml: &yaml_rust2::Yaml, path: &str) -> Vec<String> {
    use yaml_rust2::Yaml;
    rust_caches(yaml, path)
        .into_iter()
        .filter(|(_, step)| !match &step["with"]["save-if"] {
            Yaml::Boolean(save) => !save,
            Yaml::String(condition) => {
                condition == "false"
                    || condition.contains("refs/heads/main")
                    || condition == "${{ inputs.save-if }}"
            }
            _ => false,
        })
        .map(|(path, _)| path)
        .collect()
}

/// The rust-cache steps of `yaml` not keyed by the build: rust-cache used outside the
/// repository's action (`in_action`: `yaml` is that action), which first leaves only the pinned
/// toolchain; a step that turns rust-cache's environment hash off, which takes the lockfile's
/// with it; and a `key` that a `shared-key` makes rust-cache ignore. By their path from the root.
fn caches_not_keyed_by_the_build(
    yaml: &yaml_rust2::Yaml,
    path: &str,
    in_action: bool,
) -> Vec<String> {
    rust_caches(yaml, path)
        .into_iter()
        .filter(|(_, step)| {
            let with = &step["with"];
            let direct = step["uses"]
                .as_str()
                .is_some_and(|uses| uses.starts_with("Swatinem/"))
                && !in_action;
            let unhashed = !with["add-rust-environment-hash-key"].is_badvalue();
            let ignored_key = !with["shared-key"].is_badvalue() && !with["key"].is_badvalue();
            direct || unhashed || ignored_key
        })
        .map(|(path, _)| path)
        .collect()
}

/// The repository's caches share 10 GB, and main's alone take about 9 (2026-11-20). The Fuzz
/// workflow saved six caches of 200 MB from every pull request that ran it, the total reached
/// 10.7 GB, and GitHub evicted main's: CI's check found no cache and rebuilt everything. Only main
/// saves a Rust cache.
#[test]
fn only_main_saves_a_rust_cache() {
    let parse = |text: &str| {
        yaml_rust2::YamlLoader::load_from_str(text)
            .unwrap()
            .remove(0)
    };
    let steps = parse(
        "jobs:
  a:
    steps:
      - uses: Swatinem/rust-cache@abc
      - uses: Swatinem/rust-cache@abc
        with:
          save-if: ${{ github.ref == 'refs/heads/main' }}
      - uses: Swatinem/rust-cache@abc
        with:
          save-if: false
      - uses: Swatinem/rust-cache@abc
        with:
          save-if: ${{ matrix.os == 'x' }}
      - uses: actions/checkout@abc
",
    );
    assert_eq!(
        caches_saved_from_any_branch(&steps, ""),
        [".jobs.a.steps[0]", ".jobs.a.steps[3]"]
    );
    for path in workflow_files() {
        let workflow = parse(&std::fs::read_to_string(&path).unwrap());
        let any = caches_saved_from_any_branch(&workflow, "");
        assert!(
            any.is_empty(),
            "{} saves a Rust cache from any branch at {any:?}",
            path.display()
        );
    }
}

/// 2026-11-20: rust-cache's environment hash takes every toolchain `rustup toolchain list` shows,
/// and GitHub's runner images came with and without a Rust 1.99.1 of their own beside the pinned
/// 1.100.0: each gave its own key, so a run on the other image restored nothing. #301 turned that
/// hash off, which turned the lockfile's off too: one key per job, found whole on every run, so
/// never saved again, and a cache that only grew staler. Every job caches through the
/// repository's action, which leaves the runner the pinned toolchain alone and keeps both hashes.
#[test]
fn rust_caches_are_keyed_by_the_build() {
    let parse = |text: &str| {
        yaml_rust2::YamlLoader::load_from_str(text)
            .unwrap()
            .remove(0)
    };
    let steps = parse(
        "jobs:
  a:
    steps:
      - uses: Swatinem/rust-cache@abc
        with:
          shared-key: check
      - uses: ./.github/actions/rust-cache
        with:
          shared-key: check
      - uses: ./.github/actions/rust-cache
        with:
          shared-key: check
          add-rust-environment-hash-key: false
      - uses: ./.github/actions/rust-cache
        with:
          shared-key: check
          key: linux
",
    );
    assert_eq!(
        caches_not_keyed_by_the_build(&steps, "", false),
        [".jobs.a.steps[0]", ".jobs.a.steps[2]", ".jobs.a.steps[3]"]
    );
    assert_eq!(
        caches_not_keyed_by_the_build(&steps, "", true),
        [".jobs.a.steps[2]", ".jobs.a.steps[3]"],
        "the action itself uses rust-cache"
    );
    for path in workflow_files() {
        let workflow = parse(&std::fs::read_to_string(&path).unwrap());
        let in_action = path.ends_with("actions/rust-cache/action.yml");
        let keyed = caches_not_keyed_by_the_build(&workflow, "", in_action);
        assert!(
            keyed.is_empty(),
            "{} caches Rust at {keyed:?} other than through `{RUST_CACHE_ACTION}` with its keys",
            path.display()
        );
    }
    // The action leaves the pinned toolchain alone before rust-cache hashes the toolchains.
    let action = parse(
        &std::fs::read_to_string(crate::util::root().join(".github/actions/rust-cache/action.yml"))
            .unwrap(),
    );
    let steps = action["runs"]["steps"]
        .as_vec()
        .expect("the action's steps");
    let prune = steps
        .iter()
        .position(|step| {
            step["run"]
                .as_str()
                .is_some_and(|run| run.contains("rustup toolchain uninstall"))
        })
        .expect("a step that uninstalls the other toolchains");
    let cache = steps
        .iter()
        .position(|step| {
            step["uses"]
                .as_str()
                .is_some_and(|uses| uses.starts_with("Swatinem/rust-cache@"))
        })
        .expect("rust-cache");
    assert!(
        prune < cache,
        "the toolchains go before rust-cache hashes them"
    );
}

/// The repository's cache of the vendored GPUI's build (`.github/actions/vendor-gpui`).
const VENDOR_GPUI_ACTION: &str = "./.github/actions/vendor-gpui";

/// The steps of `workflow` that use the vendored GPUI's cache apart from the job's Rust cache:
/// a restore with no Rust cache restored before it (rust-cache cleans the target folder on a
/// partial match), and a save whose `rust-cache-hit` is not that Rust cache's `cache-hit` or
/// whose `restored` is not that restore's. By their path from the root.
fn vendor_builds_apart_from_their_rust_cache(workflow: &yaml_rust2::Yaml) -> Vec<String> {
    use yaml_rust2::Yaml;
    let mut apart = Vec::new();
    let Some(jobs) = workflow["jobs"].as_hash() else {
        return apart;
    };
    for (job, body) in jobs {
        let job = job.as_str().unwrap_or_default();
        let (mut rust_caches, mut restores) = (Vec::new(), Vec::new());
        for (index, step) in body["steps"].as_vec().into_iter().flatten().enumerate() {
            let uses = step["uses"].as_str().unwrap_or_default();
            let id = step["id"].as_str();
            if uses == RUST_CACHE_ACTION {
                rust_caches.extend(id);
                continue;
            }
            if uses != VENDOR_GPUI_ACTION {
                continue;
            }
            let with = &step["with"];
            let saves = matches!(&with["save"], Yaml::Boolean(true))
                || with["save"].as_str() == Some("true");
            let output_of = |ids: &[&str], input: &str, output: &str| {
                with[input].as_str().is_some_and(|value| {
                    ids.iter()
                        .any(|id| value == format!("${{{{ steps.{id}.outputs.{output} }}}}"))
                })
            };
            let together = if saves {
                output_of(&rust_caches, "rust-cache-hit", "cache-hit")
                    && output_of(&restores, "restored", "restored")
            } else {
                restores.extend(id);
                !rust_caches.is_empty() && id.is_some()
            };
            if !together {
                apart.push(format!(".jobs.{job}.steps[{index}]"));
            }
        }
    }
    apart
}

/// 2026-11-20: the vendored GPUI's build sits between crates of the job's Rust cache, under it
/// (`gpui-pre-util`) and over it (`gpui-base`), and Cargo rebuilds a crate when one it depends on
/// was built after it ("the dependency `gpui-pre-util` was rebuilt"). Its cache was keyed by its
/// sources alone, so it outlived the Rust caches that new manifests and keys replaced (#301,
/// #307, #309): two hours older than them, it made every job rebuild GPUI with both caches
/// found, 2 minutes on each of CI's two longest jobs. Each job saves it whenever it saves its
/// Rust cache, and the Rust cache is keyed by its sources.
#[test]
fn vendor_gpui_is_saved_with_its_rust_cache() {
    let parse = |text: &str| {
        yaml_rust2::YamlLoader::load_from_str(text)
            .unwrap()
            .remove(0)
    };
    let workflow = parse(
        "jobs:
  together:
    steps:
      - id: cache
        uses: ./.github/actions/rust-cache
      - id: vendor
        uses: ./.github/actions/vendor-gpui
      - uses: ./.github/actions/vendor-gpui
        with:
          save: true
          restored: ${{ steps.vendor.outputs.restored }}
          rust-cache-hit: ${{ steps.cache.outputs.cache-hit }}
  apart:
    steps:
      - id: vendor
        uses: ./.github/actions/vendor-gpui
      - id: cache
        uses: ./.github/actions/rust-cache
      - uses: ./.github/actions/vendor-gpui
        with:
          save: true
          restored: ${{ steps.vendor.outputs.restored }}
      - uses: ./.github/actions/vendor-gpui
        with:
          save: true
          restored: ${{ steps.other.outputs.restored }}
          rust-cache-hit: ${{ steps.cache.outputs.cache-hit }}
",
    );
    assert_eq!(
        vendor_builds_apart_from_their_rust_cache(&workflow),
        [
            ".jobs.apart.steps[0]",
            ".jobs.apart.steps[2]",
            ".jobs.apart.steps[3]"
        ]
    );
    let mut users = 0;
    for path in workflow_files() {
        let text = std::fs::read_to_string(&path).unwrap();
        users += text.matches(VENDOR_GPUI_ACTION).count();
        let apart = vendor_builds_apart_from_their_rust_cache(&parse(&text));
        assert!(
            apart.is_empty(),
            "{} uses the vendored GPUI's cache apart from its job's Rust cache at {apart:?}",
            path.display()
        );
    }
    assert!(users > 0, "some job caches the vendored GPUI's build");

    let action = |name: &str| {
        parse(
            &std::fs::read_to_string(
                crate::util::root().join(format!(".github/actions/{name}/action.yml")),
            )
            .unwrap(),
        )
    };
    // It saves whenever the Rust cache saves, under a key of the run's own: one of an earlier
    // run's exists, and a cache's key is never replaced.
    let vendor = action("vendor-gpui");
    let save = vendor["runs"]["steps"]
        .as_vec()
        .into_iter()
        .flatten()
        .find(|step| {
            step["uses"]
                .as_str()
                .is_some_and(|uses| uses.starts_with("actions/cache/save@"))
        })
        .expect("the vendored build's save");
    assert!(
        save["if"]
            .as_str()
            .is_some_and(|condition| condition.contains("inputs.rust-cache-hit != 'true'")),
        "the vendored build is saved when the Rust cache is"
    );
    assert!(
        save["with"]["key"]
            .as_str()
            .is_some_and(|key| key.contains("github.run_id")),
        "the vendored build's key is the run's own"
    );
    // The Rust cache's key holds the vendored sources.
    let rust_cache = action("rust-cache");
    let cache = rust_cache["runs"]["steps"]
        .as_vec()
        .into_iter()
        .flatten()
        .find(|step| {
            step["uses"]
                .as_str()
                .is_some_and(|uses| uses.starts_with("Swatinem/rust-cache@"))
        })
        .expect("rust-cache");
    assert_eq!(
        cache["env"]["KCHESS_VENDORED_GPUI"].as_str(),
        Some("${{ hashFiles('vendor/**') }}"),
        "rust-cache is given the vendored sources' hash"
    );
    assert!(
        cache["with"]["env-vars"].as_str().is_some_and(|vars| vars
            .split_whitespace()
            .any(|var| var == "KCHESS_VENDORED_GPUI")),
        "rust-cache hashes the vendored sources into its key"
    );
}

/// The folders of the repository's own packages that xtask's program is built from (its normal
/// and build dependencies, on any platform), relative to the workspace root, from `cargo
/// metadata`.
fn xtask_sources(metadata: &serde_json::Value) -> Vec<String> {
    use std::collections::{BTreeSet, HashMap};
    let root = std::path::Path::new(metadata["workspace_root"].as_str().unwrap_or_default());
    let manifests: HashMap<&str, &str> = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|package| Some((package["id"].as_str()?, package["manifest_path"].as_str()?)))
        .collect();
    let built_from: HashMap<&str, Vec<&str>> = metadata["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| {
            let deps = node["deps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|dep| {
                    dep["dep_kinds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|kind| {
                            kind["kind"].is_null() || kind["kind"].as_str() == Some("build")
                        })
                })
                .filter_map(|dep| dep["pkg"].as_str())
                .collect();
            Some((node["id"].as_str()?, deps))
        })
        .collect();
    let mut todo: Vec<&str> = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|package| package["name"].as_str() == Some("xtask"))
        .filter_map(|package| package["id"].as_str())
        .collect();
    let mut seen = BTreeSet::new();
    while let Some(id) = todo.pop() {
        if seen.insert(id) {
            todo.extend(built_from.get(id).into_iter().flatten());
        }
    }
    let mut folders: Vec<String> = seen
        .into_iter()
        .filter_map(|id| {
            let folder = std::path::Path::new(manifests.get(id)?).parent()?;
            let relative = folder.strip_prefix(root).ok()?;
            Some(
                relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
            )
        })
        .collect();
    folders.sort();
    folders
}

/// The `hashFiles` patterns in `key`, in their order.
fn hashed_files(key: &str) -> Vec<String> {
    key.split("hashFiles(")
        .skip(1)
        .flat_map(|call| {
            call.split(')')
                .next()
                .unwrap_or_default()
                .split(',')
                .map(|pattern| pattern.trim().trim_matches('\'').to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// CI's `changes` job, which every other job waits for, runs xtask's program from a cache kept
/// by the sources it is built from (2026-11-20: building it from the check cache took 26 of the
/// job's 36 seconds). A crate xtask comes to build on and the key leaves out would let a pull
/// request that changes it run the program main built: its key names every one.
#[test]
fn xtask_program_is_keyed_by_its_sources() {
    let metadata = serde_json::json!({
        "workspace_root": "/repo",
        "packages": [
            {"id": "xtask", "name": "xtask", "manifest_path": "/repo/tooling/xtask/Cargo.toml"},
            {"id": "used", "name": "used", "manifest_path": "/repo/crates/used/Cargo.toml"},
            {"id": "deeper", "name": "deeper", "manifest_path": "/repo/crates/deeper/Cargo.toml"},
            {"id": "tests", "name": "tests", "manifest_path": "/repo/crates/tests/Cargo.toml"},
            {"id": "serde", "name": "serde", "manifest_path": "/home/.cargo/registry/serde/Cargo.toml"}
        ],
        "resolve": {"nodes": [
            {"id": "xtask", "deps": [
                {"pkg": "used", "dep_kinds": [{"kind": null}]},
                {"pkg": "serde", "dep_kinds": [{"kind": null}]},
                {"pkg": "tests", "dep_kinds": [{"kind": "dev"}]}
            ]},
            {"id": "used", "deps": [{"pkg": "deeper", "dep_kinds": [{"kind": "build"}]}]},
            {"id": "deeper", "deps": []},
            {"id": "tests", "deps": []},
            {"id": "serde", "deps": []}
        ]}
    });
    assert_eq!(
        xtask_sources(&metadata),
        ["crates/deeper", "crates/used", "tooling/xtask"]
    );
    assert_eq!(
        hashed_files("x-${{ runner.os }}-${{ hashFiles('a/**', 'Cargo.lock') }}"),
        ["a/**", "Cargo.lock"]
    );

    let text =
        crate::util::output("cargo", &["metadata", "--format-version", "1", "--locked"]).unwrap();
    let sources = xtask_sources(&serde_json::from_str(&text).unwrap());
    assert!(
        sources.contains(&"tooling/xtask".to_string()),
        "{sources:?}"
    );
    let ci = yaml_rust2::YamlLoader::load_from_str(
        &std::fs::read_to_string(crate::util::root().join(".github/workflows/ci.yml")).unwrap(),
    )
    .unwrap()
    .remove(0);
    let steps = ci["jobs"]["changes"]["steps"]
        .as_vec()
        .expect("the changes job's steps");
    let restore = steps
        .iter()
        .find(|step| step["id"].as_str() == Some("xtask"))
        .expect("the changes job restores xtask's program");
    let hashed = hashed_files(restore["with"]["key"].as_str().unwrap_or_default());
    let missing: Vec<String> = sources
        .iter()
        .map(|folder| format!("{folder}/**"))
        .chain(
            [
                "Cargo.toml",
                "Cargo.lock",
                "rust-toolchain.toml",
                ".cargo/config.toml",
            ]
            .map(String::from),
        )
        .filter(|pattern| !hashed.contains(pattern))
        .collect();
    assert!(
        missing.is_empty(),
        "the key of xtask's program leaves out {missing:?}"
    );
}

#[test]
fn bash_steps_are_found_with_their_shell() {
    let text = "jobs:\n  a:\n    runs-on: macos-latest\n    steps:\n      - name: One\n        run: |\n          echo \"$(date)\"\n          echo two\n      - run: echo three\n";
    let steps = bash_steps(text);
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].1, "echo \"$(date)\"\necho two");
    assert_eq!(steps[1].1, "echo three");
    let windows = "jobs:\n  a:\n    runs-on: windows-latest\n    steps:\n      - run: Get-Item x\n      - shell: bash\n        run: echo bash\n";
    assert_eq!(bash_steps(windows), vec![(7, "echo bash".to_string())]);
}

/// What CI's `changes` job decides for a commit: its `docs` step's script from ci.yml, run in
/// `repo` for `event` on `reference` from `base`, with xtask's `scope` answering `code=true`.
#[cfg(unix)]
fn release_decision(repo: &std::path::Path, event: &str, reference: &str, base: &str) -> String {
    let text =
        std::fs::read_to_string(crate::util::root().join(".github/workflows/ci.yml")).unwrap();
    let workflow = &yaml_rust2::YamlLoader::load_from_str(&text).unwrap()[0];
    let script = workflow["jobs"]["changes"]["steps"]
        .as_vec()
        .unwrap()
        .iter()
        .find(|step| step["id"].as_str() == Some("docs"))
        .and_then(|step| step["run"].as_str())
        .unwrap();
    let xtask = repo.join("target/debug/xtask");
    std::fs::create_dir_all(xtask.parent().unwrap()).unwrap();
    std::fs::write(&xtask, "#!/bin/sh\necho code=true\necho packaging=false\n").unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&xtask, std::fs::Permissions::from_mode(0o755)).unwrap();
    let outputs = repo.join("outputs");
    std::fs::write(&outputs, "").unwrap();
    let pr_base = if event == "pull_request" { base } else { "" };
    let out = std::process::Command::new("bash")
        .args(["-e", "-c", script])
        .current_dir(repo)
        .env("BASE", base)
        .env("PR_BASE", pr_base)
        .env("EVENT", event)
        .env("GITHUB_REF", reference)
        .env("GITHUB_OUTPUT", &outputs)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read_to_string(&outputs)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("release="))
        .unwrap()
        .to_string()
}

/// A push to main packages as a release while its version has no tag: 2026.11.8's
/// release-verify fix merged after the version change, CI skipped its packaging, and
/// release-verify refused the only commit that had the fix.
#[cfg(unix)]
#[test]
fn a_push_of_an_untagged_version_is_a_release_commit() {
    let repo = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| crate::util::git(repo.path(), args, &[0]).unwrap();
    let manifest = |version: &str| {
        let text = format!("[workspace.package]\nversion = \"{version}\"\n\n[workspace]\n");
        std::fs::write(repo.path().join("Cargo.toml"), text).unwrap();
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "ci@example.com"]);
    git(&["config", "user.name", "CI"]);
    manifest("2026.11.7");
    git(&["add", "Cargo.toml"]);
    git(&["commit", "-q", "-m", "2026.11.7"]);
    git(&["tag", "v2026.11.7"]);
    let released = git(&["rev-parse", "HEAD"]).trim().to_string();
    manifest("2026.11.8");
    git(&["commit", "-q", "-am", "Release 2026.11.8"]);
    let bump = git(&["rev-parse", "HEAD"]).trim().to_string();
    std::fs::write(repo.path().join("fix.rs"), "").unwrap();
    git(&["add", "fix.rs"]);
    git(&["commit", "-q", "-m", "A fix after the version change"]);
    let main = "refs/heads/main";
    // The version change, pushed or in its pull request.
    assert_eq!(
        release_decision(repo.path(), "push", main, &released),
        "true"
    );
    assert_eq!(
        release_decision(repo.path(), "pull_request", "refs/pull/1/merge", &released),
        "true"
    );
    // A later push while 2026.11.8 has no tag; not its pull request, nor another branch.
    assert_eq!(release_decision(repo.path(), "push", main, &bump), "true");
    assert_eq!(
        release_decision(repo.path(), "pull_request", "refs/pull/2/merge", &bump),
        "false"
    );
    assert_eq!(
        release_decision(repo.path(), "push", "refs/heads/other", &bump),
        "false"
    );
    // Once tagged, a push to main is not a release commit.
    git(&["tag", "v2026.11.8", &bump]);
    assert_eq!(release_decision(repo.path(), "push", main, &bump), "false");
}
