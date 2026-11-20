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
fn rust_caches<'a>(yaml: &'a yaml_rust2::Yaml, path: &str) -> Vec<(String, &'a yaml_rust2::Yaml)> {
    use yaml_rust2::Yaml;
    match yaml {
        Yaml::Hash(map) => {
            let mut found = Vec::new();
            if yaml["uses"]
                .as_str()
                .is_some_and(|uses| uses.starts_with("Swatinem/rust-cache@"))
            {
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
/// `save-if`, or one that does not name main (`false` saves nothing). By their path from the root.
fn caches_saved_from_any_branch(yaml: &yaml_rust2::Yaml, path: &str) -> Vec<String> {
    use yaml_rust2::Yaml;
    rust_caches(yaml, path)
        .into_iter()
        .filter(|(_, step)| !match &step["with"]["save-if"] {
            Yaml::Boolean(save) => !save,
            Yaml::String(condition) => {
                condition == "false" || condition.contains("refs/heads/main")
            }
            _ => false,
        })
        .map(|(path, _)| path)
        .collect()
}

/// The `rust-cache` steps of `yaml` whose key depends on the runner rather than on the build:
/// those that keep rust-cache's environment hash, which takes every toolchain the runner image
/// installed besides the pinned one, and those whose `key` a `shared-key` makes rust-cache ignore.
fn caches_keyed_by_the_runner(yaml: &yaml_rust2::Yaml, path: &str) -> Vec<String> {
    use yaml_rust2::Yaml;
    rust_caches(yaml, path)
        .into_iter()
        .filter(|(_, step)| {
            let with = &step["with"];
            let hashes_the_runner =
                !matches!(&with["add-rust-environment-hash-key"], Yaml::Boolean(false))
                    && with["add-rust-environment-hash-key"].as_str() != Some("false");
            let ignored_key = !with["shared-key"].is_badvalue() && !with["key"].is_badvalue();
            hashes_the_runner || ignored_key
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
/// 1.100.0. Each gave its own key, so a run on the other image restored nothing and compiled every
/// crate: 11 of 15 jobs in five pull requests. The pinned toolchain is in the manifests' hash.
#[test]
fn rust_caches_are_keyed_by_the_build_alone() {
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
      - uses: Swatinem/rust-cache@abc
        with:
          shared-key: check
          add-rust-environment-hash-key: false
      - uses: Swatinem/rust-cache@abc
        with:
          shared-key: check
          key: linux
          add-rust-environment-hash-key: false
      - uses: Swatinem/rust-cache@abc
        with:
          key: linux
          add-rust-environment-hash-key: 'false'
",
    );
    assert_eq!(
        caches_keyed_by_the_runner(&steps, ""),
        [".jobs.a.steps[0]", ".jobs.a.steps[2]"]
    );
    for path in workflow_files() {
        let workflow = parse(&std::fs::read_to_string(&path).unwrap());
        let keyed = caches_keyed_by_the_runner(&workflow, "");
        assert!(
            keyed.is_empty(),
            "{} keys a Rust cache by its runner at {keyed:?}: set \
             `add-rust-environment-hash-key: false`, and drop `key` beside `shared-key`",
            path.display()
        );
    }
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
