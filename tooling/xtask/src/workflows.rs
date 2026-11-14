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

/// The Mutants workflow started by hand offers every member but the workspace hack: its list
/// was written by hand and lacked kchess-delta, the crate installed apps read updates with.
#[test]
fn mutants_can_be_run_on_every_member() {
    let path = crate::util::root().join(".github/workflows/mutants.yml");
    let text = std::fs::read_to_string(path).unwrap();
    let workflow = &yaml_rust2::YamlLoader::load_from_str(&text).unwrap()[0];
    let mut offered: Vec<&str> =
        workflow["on"]["workflow_dispatch"]["inputs"]["package"]["options"]
            .as_vec()
            .expect("the package choices")
            .iter()
            .filter_map(yaml_rust2::Yaml::as_str)
            .collect();
    offered.sort_unstable();
    let mut members = crate::check::members().unwrap();
    members.retain(|name| crate::members::role(name) != crate::members::Role::Hack);
    members.sort_unstable();
    assert_eq!(offered, members);
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
