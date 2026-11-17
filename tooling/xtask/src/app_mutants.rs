//! The weekly run of the app's mutants (`mutants.yml`'s `gpui` jobs), each tested only by the
//! tests that run its line.
//!
//! Every app mutant used to run the app's whole suite: 669 tests, about 2 minutes on a CI runner,
//! though a mutant's change can only show in a test that runs the changed line. A week's 4,000
//! mutants needed some 150 runner-hours, and the job's shards were cancelled at six hours with
//! nothing reported (run 43584561477, 2026-11-14). So the plan (`--app-plan`) builds the app's
//! tests with coverage once, runs each test alone, and gives each mutant the tests that ran a
//! line of its span: half the mutants need 5 tests or fewer. A mutant that no test runs is
//! reported as uncovered, without being built. The mutants that share their tests are tested
//! together, in chunks the plan spreads over as many shards as their estimated time needs; each
//! shard (`--app-shard <k>`) runs cargo-mutants once per chunk, with a nextest filter of the
//! chunk's tests. See docs/mutation-testing.md.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::members::{APP, APP_FILES};
use crate::util::{Result, say, without_git_location};

/// The plan the plan job writes and each shard reads, under the (ignored) target directory.
pub const PLAN: &str = "target/mutants-app-plan.json";

/// The app's test binary (its `[[bin]]`'s name).
const TEST_BINARY: &str = "KChess";

/// What a mutant costs besides its tests: an incremental rebuild of the app, 17 s on average on
/// a CI runner (run 43584561477), and cargo-mutants' own steps.
const BUILD_SECONDS: f64 = 20.0;

/// The estimated work a shard is planned for: four hours, leaving room within the job's six for
/// its setup, its first build (about seven minutes) and estimates that run long.
const SHARD_SECONDS: f64 = 4.0 * 3600.0;

/// The most shards the plan asks for. The app's run has its own day (Sunday) so these do not
/// wait behind the rules' eight on Saturday; GitHub runs twenty of an account's jobs at once.
const MAX_SHARDS: usize = 16;

/// The estimated time at which a chunk is split: a shard checks its deadline between chunks.
const CHUNK_SECONDS: f64 = 15.0 * 60.0;

/// When a shard stops starting chunks: five hours after it started, so what it tested is
/// reported within the job's six, and what it did not is named (`untested.txt`) rather than lost
/// to a cancelled job.
const SHARD_DEADLINE: Duration = Duration::from_secs(5 * 3600);

/// How long one test may run while coverage is measured, past which it is stopped (nextest's own
/// limit, `.config/nextest.toml`).
const TEST_LIMIT: Duration = Duration::from_secs(300);

/// A mutant of the app, as `cargo mutants --list --json` names it, with its span's lines.
#[derive(Debug, Clone, PartialEq)]
struct Mutant {
    name: String,
    file: String,
    first: u32,
    last: u32,
}

/// The mutants of `cargo mutants --list --json`'s output.
fn mutants_listed(json: &str) -> Result<Vec<Mutant>> {
    let list: Value =
        serde_json::from_str(json).map_err(|cause| format!("cargo mutants --list: {cause}"))?;
    let entries = list
        .as_array()
        .ok_or("cargo mutants --list --json gave no list")?;
    entries
        .iter()
        .map(|entry| {
            let text = |key: &str| entry[key].as_str().map(str::to_string);
            let line = |end: &str| {
                entry["span"][end]["line"]
                    .as_u64()
                    .and_then(|line| u32::try_from(line).ok())
            };
            match (text("name"), text("file"), line("start"), line("end")) {
                (Some(name), Some(file), Some(first), Some(last)) => Ok(Mutant {
                    name,
                    file,
                    first,
                    last,
                }),
                _ => Err(format!(
                    "A listed mutant without a name, file or span: {entry}"
                )),
            }
        })
        .collect()
}

/// The lines an lcov report (`llvm-cov export -format=lcov`) says ran, by file relative to
/// `repo`, `/`-separated.
fn covered_lines(lcov: &str, repo: &Path) -> BTreeMap<String, BTreeSet<u32>> {
    let mut covered: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut file: Option<String> = None;
    for line in lcov.lines() {
        if let Some(path) = line.strip_prefix("SF:") {
            let path = Path::new(path);
            let relative = path.strip_prefix(repo).unwrap_or(path);
            file = Some(relative.to_string_lossy().replace('\\', "/"));
        } else if let (Some(file), Some(data)) = (&file, line.strip_prefix("DA:")) {
            let mut fields = data.split(',');
            let number = fields.next().and_then(|n| n.parse::<u32>().ok());
            let count = fields.next().and_then(|n| n.parse::<u64>().ok());
            if let (Some(number), Some(count)) = (number, count)
                && count > 0
            {
                covered.entry(file.clone()).or_default().insert(number);
            }
        }
    }
    covered
}

/// Which tests ran each line: file, then line, then the tests' indexes.
#[derive(Default)]
struct Index(HashMap<String, HashMap<u32, BTreeSet<usize>>>);

impl Index {
    fn add(&mut self, test: usize, covered: &BTreeMap<String, BTreeSet<u32>>) {
        for (file, lines) in covered {
            let by_line = self.0.entry(file.clone()).or_default();
            for line in lines {
                by_line.entry(*line).or_default().insert(test);
            }
        }
    }

    /// The tests that ran a line of `mutant`'s span: only they can see it.
    fn tests_of(&self, mutant: &Mutant) -> BTreeSet<usize> {
        let Some(by_line) = self.0.get(&mutant.file) else {
            return BTreeSet::new();
        };
        (mutant.first..=mutant.last)
            .filter_map(|line| by_line.get(&line))
            .flatten()
            .copied()
            .collect()
    }
}

/// How long `tests` take on a runner with `cpus` processors, from each one's `seconds`: as long
/// as the longest, or as their total spread over the processors, whichever is longer.
fn wall(tests: &BTreeSet<usize>, seconds: &[f64], cpus: usize) -> f64 {
    let each = tests.iter().map(|test| seconds[*test]);
    let longest = each.clone().fold(0.0, f64::max);
    let total: f64 = each.sum();
    longest.max(total / cpus.max(1) as f64)
}

/// Mutants tested together: by one run of cargo-mutants, with the same tests.
#[derive(Debug, Clone, PartialEq)]
struct Chunk {
    mutants: Vec<String>,
    /// The first line of each mutant's span, by file: what the chunk's diff marks.
    lines: BTreeMap<String, BTreeSet<u32>>,
    tests: BTreeSet<usize>,
    estimate: f64,
    shard: usize,
}

/// The chunks that test `mutants` (each with the tests that see it), and the mutants no test
/// runs. Mutants with the same tests go together, in the order listed, split where a chunk
/// would pass `CHUNK_SECONDS`.
fn chunks(
    mutants: &[(Mutant, BTreeSet<usize>)],
    seconds: &[f64],
    cpus: usize,
) -> (Vec<Chunk>, Vec<String>) {
    let mut uncovered = Vec::new();
    let mut groups: Vec<(BTreeSet<usize>, Vec<&Mutant>)> = Vec::new();
    let mut group_of: HashMap<&BTreeSet<usize>, usize> = HashMap::new();
    for (mutant, tests) in mutants {
        if tests.is_empty() {
            uncovered.push(mutant.name.clone());
            continue;
        }
        match group_of.get(tests) {
            Some(at) => groups[*at].1.push(mutant),
            None => {
                group_of.insert(tests, groups.len());
                groups.push((tests.clone(), vec![mutant]));
            }
        }
    }
    let mut out = Vec::new();
    for (tests, members) in groups {
        let each = BUILD_SECONDS + wall(&tests, seconds, cpus);
        let per_chunk = ((CHUNK_SECONDS / each) as usize).max(1);
        for part in members.chunks(per_chunk) {
            out.push(Chunk {
                mutants: part.iter().map(|mutant| mutant.name.clone()).collect(),
                lines: part.iter().fold(BTreeMap::new(), |mut lines, mutant| {
                    lines
                        .entry(mutant.file.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(mutant.first);
                    lines
                }),
                tests: tests.clone(),
                estimate: each * part.len() as f64,
                shard: 0,
            });
        }
    }
    (out, uncovered)
}

/// How many shards `seconds` of estimated work take: `SHARD_SECONDS` each, at most `MAX_SHARDS`,
/// none for none.
fn shard_count(seconds: f64) -> usize {
    if seconds <= 0.0 {
        return 0;
    }
    ((seconds / SHARD_SECONDS).ceil() as usize).clamp(1, MAX_SHARDS)
}

/// Spreads `chunks` over `shards`: the longest first, each to the shard with the least so far.
fn assign(chunks: &mut [Chunk], shards: usize) {
    let mut order: Vec<usize> = (0..chunks.len()).collect();
    order.sort_by(|a, b| chunks[*b].estimate.total_cmp(&chunks[*a].estimate));
    let mut load = vec![0.0_f64; shards.max(1)];
    for at in order {
        let lightest = (0..load.len())
            .min_by(|a, b| load[*a].total_cmp(&load[*b]))
            .unwrap_or(0);
        load[lightest] += chunks[at].estimate;
        chunks[at].shard = lightest;
    }
}

/// The plan as the shards read it.
fn plan_json(
    base: &str,
    shards: usize,
    chunks: &[Chunk],
    uncovered: &[String],
    tests: &[String],
) -> Value {
    json!({
        "base": base,
        "shards": shards,
        "uncovered": uncovered,
        "chunks": chunks.iter().map(|chunk| json!({
            "shard": chunk.shard,
            "estimate": chunk.estimate.round(),
            "lines": chunk.lines,
            "mutants": chunk.mutants,
            "tests": chunk.tests.iter().map(|test| &tests[*test]).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// The GitHub step outputs of a plan: how many mutants, and the shards as a JSON list for the
/// job matrix.
fn outputs(count: usize, shards: usize) -> String {
    let numbers: Vec<String> = (0..shards).map(|shard| shard.to_string()).collect();
    format!("count={count}\nshards=[{}]\n", numbers.join(","))
}

/// A chunk of a plan, as a shard reads it.
#[derive(Debug, PartialEq)]
struct Planned {
    estimate: f64,
    lines: BTreeMap<String, Vec<u32>>,
    mutants: Vec<String>,
    tests: Vec<String>,
}

/// The chunks of `plan` that `shard` tests, and the mutants it reports as uncovered (all of them
/// by shard 0, so the shards together name each once).
fn planned_for(plan: &Value, shard: usize) -> Result<(Vec<Planned>, Vec<String>)> {
    let strings = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let chunks = plan["chunks"]
        .as_array()
        .ok_or("The plan has no chunks: was it written by `--app-plan`?")?;
    let mine = chunks
        .iter()
        .filter(|chunk| chunk["shard"].as_u64() == Some(shard as u64))
        .map(|chunk| Planned {
            estimate: chunk["estimate"].as_f64().unwrap_or(0.0),
            lines: chunk["lines"]
                .as_object()
                .map(|files| {
                    files
                        .iter()
                        .map(|(file, lines)| {
                            let numbers = lines.as_array().map_or_else(Vec::new, |lines| {
                                lines
                                    .iter()
                                    .filter_map(|line| line.as_u64())
                                    .filter_map(|line| u32::try_from(line).ok())
                                    .collect()
                            });
                            (file.clone(), numbers)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            mutants: strings(&chunk["mutants"]),
            tests: strings(&chunk["tests"]),
        })
        .collect();
    let uncovered = if shard == 0 {
        strings(&plan["uncovered"])
    } else {
        Vec::new()
    };
    Ok((mine, uncovered))
}

/// `name` as a regular expression that matches it alone.
fn exact_re(name: &str) -> String {
    let mut out = String::from("^");
    for c in name.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('$');
    out
}

/// A nextest filterset that runs exactly `tests`.
fn filterset(tests: &[String]) -> String {
    tests
        .iter()
        .map(|test| format!("test(={test})"))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// A diff that marks the chunk's `lines` as added, in files whose text `source` gives: with
/// `--in-diff` it holds cargo-mutants to the mutants on those lines. Its names (`--re`) cannot
/// alone: cargo-mutants 27.1 never checks a struct field's mutants against them, so every chunk
/// tested all of its files' "delete field" mutants. Each run of marked lines is one hunk, numbered
/// as if the lines were new, the old file being the source without them.
fn chunk_diff(
    lines: &BTreeMap<String, Vec<u32>>,
    source: impl Fn(&str) -> Result<String>,
) -> Result<String> {
    let mut diff = String::new();
    for (file, numbers) in lines {
        let text = source(file)?;
        let all: Vec<&str> = text.lines().collect();
        let marked: BTreeSet<u32> = numbers.iter().copied().collect();
        diff.push_str(&format!("--- a/{file}\n+++ b/{file}\n"));
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for line in marked {
            match runs.last_mut() {
                Some((_, last)) if *last + 1 == line => *last = line,
                _ => runs.push((line, line)),
            }
        }
        let mut added_before = 0;
        for (first, last) in runs {
            let count = last - first + 1;
            let old_start = first - 1 - added_before;
            diff.push_str(&format!("@@ -{old_start},0 +{first},{count} @@\n"));
            for line in first..=last {
                let content = all.get(line as usize - 1).ok_or_else(|| {
                    format!("{file} has no line {line}: is the plan of another commit?")
                })?;
                diff.push_str(&format!("+{content}\n"));
            }
            added_before += count;
        }
    }
    Ok(diff)
}

/// cargo-mutants' arguments for a chunk, its results under `out`, its diff (`chunk_diff`) at
/// `diff`: in place, built incrementally after the shard's first build, without a baseline (the
/// plan ran every test), each mutant tested by the chunk's tests alone.
fn chunk_args(chunk: &Planned, out: &Path, diff: &Path) -> Vec<String> {
    let mut args: Vec<String> = [
        "mutants",
        "--in-place",
        "--baseline",
        "skip",
        "--no-shuffle",
        "--test-tool",
        "nextest",
        "--package",
        APP,
        // A mutant whose tests run past five minutes hangs: the longest test takes well under one.
        "--timeout",
        "300",
        "--build-timeout",
        "600",
    ]
    .map(String::from)
    .to_vec();
    args.push("--output".into());
    args.push(out.to_string_lossy().into_owned());
    args.push("--in-diff".into());
    args.push(diff.to_string_lossy().into_owned());
    for file in chunk.lines.keys() {
        args.extend(["--file".to_string(), file.clone()]);
    }
    for mutant in &chunk.mutants {
        args.extend(["--re".to_string(), exact_re(mutant)]);
    }
    args.extend([
        "--cargo-test-arg".to_string(),
        "-E".to_string(),
        "--cargo-test-arg".to_string(),
        filterset(&chunk.tests),
    ]);
    args
}

/// The outcome lists cargo-mutants writes, and those a shard adds.
const OUTCOMES: [&str; 4] = ["caught", "missed", "timeout", "unviable"];

/// Appends a chunk's outcome lists (under `chunk`, cargo-mutants' `mutants.out`) to the shard's
/// (`top`).
fn merge(chunk: &Path, top: &Path) -> Result {
    for outcome in OUTCOMES {
        let name = format!("{outcome}.txt");
        let Ok(text) = std::fs::read_to_string(chunk.join(&name)) else {
            continue;
        };
        append(&top.join(&name), &text)?;
    }
    Ok(())
}

fn append(path: &Path, text: &str) -> Result {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|cause| format!("{}: {cause}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|cause| format!("{}: {cause}", path.display()))
}

/// The lines of an outcome list in `top`.
fn count_in(top: &Path, outcome: &str) -> usize {
    std::fs::read_to_string(top.join(format!("{outcome}.txt")))
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}

/// What a shard's run ends with: nothing when every mutant it tested was caught or unviable,
/// else what to look at.
fn verdict(missed: usize, timeouts: usize, uncovered: usize, untested: usize) -> Result {
    if missed + timeouts + uncovered + untested == 0 {
        return Ok(());
    }
    Err(format!(
        "{missed} missed, {timeouts} timed out, {uncovered} on lines no test runs, {untested} \
         not reached before the shard's deadline: mutants.out names them (missed.txt, \
         timeout.txt, uncovered.txt, untested.txt)."
    ))
}

// The plan and the shard run below build and run the app's tests and cargo-mutants on the
// repository itself; what they decide is the functions above, tested.

/// `mutants --app-plan --base <ref>`: the plan for the app's mutants on the lines changed since
/// `base` (`PLAN`), and its GitHub step outputs on standard output.
pub fn plan(repo: &Path, base: &str) -> Result {
    let started = Instant::now();
    let mutants = if crate::mutants::write_diff(repo, base)? {
        list(repo)?
    } else {
        Vec::new()
    };
    say(&format!("{} app mutants since {base}.", mutants.len()));
    let (tests, seconds, index) = if mutants.is_empty() {
        (Vec::new(), Vec::new(), Index::default())
    } else {
        coverage(repo)?
    };
    let cpus = cpus();
    let selected: Vec<(Mutant, BTreeSet<usize>)> = mutants
        .into_iter()
        .map(|mutant| {
            let tests = index.tests_of(&mutant);
            (mutant, tests)
        })
        .collect();
    let (mut chunks, uncovered) = chunks(&selected, &seconds, cpus);
    let total: f64 = chunks.iter().map(|chunk| chunk.estimate).sum();
    let shards = shard_count(total);
    assign(&mut chunks, shards);
    let path = repo.join(PLAN);
    let text = plan_json(base, shards, &chunks, &uncovered, &tests).to_string();
    std::fs::write(&path, text).map_err(|cause| format!("{}: {cause}", path.display()))?;
    say(&format!(
        "{} mutants to test in {} chunks, about {:.1} h over {shards} shards; {} on lines no \
         test runs. Planned in {} s.",
        selected.len() - uncovered.len(),
        chunks.len(),
        total / 3600.0,
        uncovered.len(),
        started.elapsed().as_secs()
    ));
    print!("{}", outputs(selected.len(), shards));
    Ok(())
}

/// The app's mutants on the diff's lines.
fn list(repo: &Path) -> Result<Vec<Mutant>> {
    let mut command = Command::new("cargo");
    let out = without_git_location(&mut command)
        .args([
            "mutants",
            "--in-diff",
            crate::mutants::DIFF,
            "--file",
            APP_FILES,
            "--list",
            "--json",
        ])
        .current_dir(repo)
        .output()
        .map_err(|cause| format!("Could not run cargo mutants: {cause}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo mutants --list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    mutants_listed(&String::from_utf8_lossy(&out.stdout))
}

fn cpus() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// Builds the app's tests with coverage and runs each alone: the tests, how long each took, and
/// which lines each ran.
fn coverage(repo: &Path) -> Result<(Vec<String>, Vec<f64>, Index)> {
    let binary = instrumented(repo)?;
    let tools = llvm_tools()?;
    let dir = repo.join("target/mutants-app-coverage");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|cause| format!("{}: {cause}", dir.display()))?;
    let tests = test_names(&binary, &dir)?;
    let package = repo.join("apps/gpui");
    let started = Instant::now();
    // The tests mostly wait on other threads and processes: twice the processors keep them busy.
    let seconds = in_parallel(tests.len(), cpus() * 2, |test| {
        run_test(
            &binary,
            &package,
            &tests[test],
            &dir.join(format!("{test}-%p.profraw")),
        )
    });
    say(&format!(
        "Ran {} tests with coverage in {} s.",
        tests.len(),
        started.elapsed().as_secs()
    ));
    let index = Mutex::new(Index::default());
    let failures = Mutex::new(Vec::new());
    in_parallel(tests.len(), cpus(), |test| {
        match lines_of(repo, &tools, &binary, &dir, test) {
            Ok(covered) => index.lock().unwrap().add(test, &covered),
            Err(cause) => failures.lock().unwrap().push(cause),
        }
        0.0
    });
    if let Some(cause) = failures.into_inner().unwrap().into_iter().next() {
        return Err(cause);
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok((tests, seconds, index.into_inner().unwrap()))
}

/// Runs `work` for `0..count`, `jobs` at once: what each returned, in order.
fn in_parallel(count: usize, jobs: usize, work: impl Fn(usize) -> f64 + Sync) -> Vec<f64> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new(vec![0.0; count]);
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::Relaxed);
                    if at >= count {
                        break;
                    }
                    let result = work(at);
                    results.lock().unwrap()[at] = result;
                }
            });
        }
    });
    results.into_inner().unwrap()
}

/// The app's test binary built with coverage of the app's own code: `cargo rustc` instruments
/// only the crate it builds, so the dependencies stay as the shards build them.
fn instrumented(repo: &Path) -> Result<PathBuf> {
    let out = Command::new("cargo")
        .args([
            "rustc",
            "--locked",
            "-p",
            APP,
            "--bin",
            TEST_BINARY,
            "--profile",
            "test",
            "--message-format",
            "json-render-diagnostics",
            "--",
            "-C",
            "instrument-coverage",
        ])
        .current_dir(repo)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|cause| format!("Could not run cargo rustc: {cause}"))?;
    if !out.status.success() {
        return Err("The app's tests did not build with coverage.".into());
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|message| message["target"]["name"] == TEST_BINARY)
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
        .ok_or_else(|| "cargo rustc named no test executable.".into())
}

/// The tests of `binary` that run by default (not `#[ignore]`d). Listing them writes a profile
/// too, into `dir` rather than the current folder.
fn test_names(binary: &Path, dir: &Path) -> Result<Vec<String>> {
    let listed = |ignored: bool| -> Result<BTreeSet<String>> {
        let mut command = Command::new(binary);
        command
            .args(["--list", "--format", "terse"])
            .env("LLVM_PROFILE_FILE", dir.join("list-%p.profraw"));
        if ignored {
            command.arg("--ignored");
        }
        let out = command
            .output()
            .map_err(|cause| format!("{}: {cause}", binary.display()))?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| line.strip_suffix(": test"))
            .map(str::to_string)
            .collect())
    };
    let ignored = listed(true)?;
    Ok(listed(false)?
        .into_iter()
        .filter(|test| !ignored.contains(test))
        .collect())
}

/// Runs one test with its coverage written to `profile`: how long it took.
fn run_test(binary: &Path, package: &Path, test: &str, profile: &Path) -> f64 {
    let started = Instant::now();
    let child = Command::new(binary)
        .args(["--exact", test, "--test-threads", "1"])
        .current_dir(package)
        .env("CARGO_MANIFEST_DIR", package)
        .env("LLVM_PROFILE_FILE", profile)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        say(&format!("{test} did not start."));
        return 0.0;
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    // Its coverage still counts: what it ran before it failed is what it sees.
                    say(&format!("{test} failed with coverage ({status})."));
                }
                break;
            }
            Ok(None) if started.elapsed() > TEST_LIMIT => {
                say(&format!("{test} stopped after {} s.", TEST_LIMIT.as_secs()));
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break,
        }
    }
    started.elapsed().as_secs_f64()
}

/// The toolchain's llvm-profdata and llvm-cov (`rustup component add llvm-tools-preview`).
fn llvm_tools() -> Result<PathBuf> {
    let sysroot = crate::util::output("rustc", &["--print", "sysroot"])?;
    let host = crate::util::output("rustc", &["-vV"])?
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        .ok_or("rustc -vV names no host")?;
    let tools = Path::new(sysroot.trim()).join(format!("lib/rustlib/{host}/bin"));
    if !tools
        .join(format!("llvm-cov{}", std::env::consts::EXE_SUFFIX))
        .exists()
    {
        return Err("llvm-cov is missing: `rustup component add llvm-tools-preview`.".into());
    }
    Ok(tools)
}

/// The app's lines test number `test` ran, from its profiles in `dir`, which it then removes.
fn lines_of(
    repo: &Path,
    tools: &Path,
    binary: &Path,
    dir: &Path,
    test: usize,
) -> Result<BTreeMap<String, BTreeSet<u32>>> {
    let prefix = format!("{test}-");
    let raws: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|cause| format!("{}: {cause}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".profraw"))
        })
        .collect();
    if raws.is_empty() {
        return Ok(BTreeMap::new());
    }
    let data = dir.join(format!("{test}.profdata"));
    let merged = Command::new(tools.join("llvm-profdata"))
        .args(["merge", "-sparse", "-o"])
        .arg(&data)
        .args(&raws)
        .status()
        .map_err(|cause| format!("llvm-profdata: {cause}"))?;
    if !merged.success() {
        return Err(format!(
            "llvm-profdata could not merge test {test}'s profiles."
        ));
    }
    let out = Command::new(tools.join("llvm-cov"))
        .args([
            "export",
            "-format=lcov",
            "-skip-functions",
            "-instr-profile",
        ])
        .arg(&data)
        .arg(binary)
        .arg(repo.join("apps/gpui/src"))
        .output()
        .map_err(|cause| format!("llvm-cov: {cause}"))?;
    if !out.status.success() {
        return Err(format!(
            "llvm-cov export failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    for raw in &raws {
        let _ = std::fs::remove_file(raw);
    }
    let _ = std::fs::remove_file(&data);
    Ok(covered_lines(&String::from_utf8_lossy(&out.stdout), repo))
}

/// `mutants --app-shard <k>`: tests shard `k`'s chunks of the plan (`PLAN`), its results in
/// `mutants.out`.
pub fn shard(repo: &Path, shard: usize) -> Result {
    let started = Instant::now();
    let path = repo.join(PLAN);
    let text =
        std::fs::read_to_string(&path).map_err(|cause| format!("{}: {cause}", path.display()))?;
    let plan: Value =
        serde_json::from_str(&text).map_err(|cause| format!("{}: {cause}", path.display()))?;
    let (chunks, uncovered) = planned_for(&plan, shard)?;
    let top = repo.join("mutants.out");
    let _ = std::fs::remove_dir_all(&top);
    std::fs::create_dir_all(&top).map_err(|cause| format!("{}: {cause}", top.display()))?;
    append(&top.join("uncovered.txt"), &lines(&uncovered))?;
    let total: f64 = chunks.iter().map(|chunk| chunk.estimate).sum();
    say(&format!(
        "Shard {shard}: {} chunks, about {:.1} h.",
        chunks.len(),
        total / 3600.0
    ));
    // The shard's first build, of every dependency: each mutant after it rebuilds only the app.
    let mut build = Command::new("cargo");
    build
        .args(["nextest", "run", "--no-run", "--locked", "--package", APP])
        .current_dir(repo)
        .env_remove("CARGO_INCREMENTAL");
    crate::util::run_with(&mut build)?;
    let mut untested = Vec::new();
    for (at, chunk) in chunks.iter().enumerate() {
        if started.elapsed() > SHARD_DEADLINE {
            untested.extend(chunk.mutants.iter().cloned());
            continue;
        }
        say(&format!(
            "Chunk {}/{}: {} mutants, {} tests, about {} s.",
            at + 1,
            chunks.len(),
            chunk.mutants.len(),
            chunk.tests.len(),
            chunk.estimate
        ));
        let out = top.join("chunks").join(at.to_string());
        std::fs::create_dir_all(&out).map_err(|cause| format!("{}: {cause}", out.display()))?;
        let mut command = Command::new("cargo");
        // As every mutants run: the run ends at a mutant's first failing test; incremental
        // builds (`mutants::in_place`).
        let diff = out.join("chunk.patch");
        let patch = chunk_diff(&chunk.lines, |file| {
            std::fs::read_to_string(repo.join(file)).map_err(|cause| format!("{file}: {cause}"))
        })?;
        std::fs::write(&diff, patch).map_err(|cause| format!("{}: {cause}", diff.display()))?;
        let status = without_git_location(&mut command)
            .args(chunk_args(chunk, &out, &diff))
            .current_dir(repo)
            .env("NEXTEST_PROFILE", crate::mutants::NEXTEST_PROFILE)
            .env_remove("CARGO_INCREMENTAL")
            .status()
            .map_err(|cause| format!("Could not run cargo mutants: {cause}"))?;
        // 0: all caught; 2: one missed; 3: one timed out. Anything else is cargo-mutants failing.
        if !matches!(status.code(), Some(0 | 2 | 3)) {
            return Err(format!("cargo mutants failed on chunk {at} ({status})."));
        }
        merge(&out.join("mutants.out"), &top)?;
    }
    append(&top.join("untested.txt"), &lines(&untested))?;
    verdict(
        count_in(&top, "missed"),
        count_in(&top, "timeout"),
        uncovered.len(),
        untested.len(),
    )
}

fn lines(items: &[String]) -> String {
    items.iter().map(|item| format!("{item}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mutant(name: &str, file: &str, first: u32, last: u32) -> Mutant {
        Mutant {
            name: name.into(),
            file: file.into(),
            first,
            last,
        }
    }

    fn set(items: &[usize]) -> BTreeSet<usize> {
        items.iter().copied().collect()
    }

    fn marked(files: &[(&str, &[u32])]) -> BTreeMap<String, BTreeSet<u32>> {
        files
            .iter()
            .map(|(file, lines)| (file.to_string(), lines.iter().copied().collect()))
            .collect()
    }

    #[test]
    fn listed_mutants_keep_their_names_and_lines() {
        let json = r#"[{"name": "apps/gpui/src/a.rs:3:5: replace f -> bool with true",
            "file": "apps/gpui/src/a.rs", "span": {"start": {"line": 3, "column": 5},
            "end": {"line": 7, "column": 6}}}]"#;
        assert_eq!(
            mutants_listed(json).unwrap(),
            [mutant(
                "apps/gpui/src/a.rs:3:5: replace f -> bool with true",
                "apps/gpui/src/a.rs",
                3,
                7
            )]
        );
        assert!(mutants_listed(r#"[{"name": "x"}]"#).is_err());
        assert!(mutants_listed("{}").is_err());
        assert!(mutants_listed("not json").is_err());
    }

    #[test]
    fn only_lines_that_ran_count_as_covered() {
        let lcov = "SF:/repo/apps/gpui/src/a.rs\nDA:3,2\nDA:4,0\nDA:5,1\nend_of_record\n\
                    SF:/repo/apps/gpui/src/b.rs\nDA:1,0\nend_of_record\n\
                    SF:/repo/apps/gpui/src/c.rs\nDA:9,1,abc\nDA:x,1\n";
        let covered = covered_lines(lcov, Path::new("/repo"));
        assert_eq!(
            covered,
            BTreeMap::from([
                ("apps/gpui/src/a.rs".to_string(), [3, 5].into()),
                ("apps/gpui/src/c.rs".to_string(), [9].into()),
            ])
        );
        // A line before any file names none.
        assert!(covered_lines("DA:1,1\n", Path::new("/repo")).is_empty());
    }

    #[test]
    fn a_mutant_is_seen_by_the_tests_that_ran_a_line_of_its_span() {
        let mut index = Index::default();
        index.add(0, &BTreeMap::from([("a.rs".to_string(), [3].into())]));
        index.add(1, &BTreeMap::from([("a.rs".to_string(), [5, 9].into())]));
        index.add(2, &BTreeMap::from([("b.rs".to_string(), [3].into())]));
        assert_eq!(index.tests_of(&mutant("m", "a.rs", 3, 5)), set(&[0, 1]));
        assert_eq!(index.tests_of(&mutant("m", "a.rs", 4, 4)), set(&[]));
        assert_eq!(index.tests_of(&mutant("m", "a.rs", 9, 9)), set(&[1]));
        assert_eq!(index.tests_of(&mutant("m", "c.rs", 3, 3)), set(&[]));
    }

    #[test]
    fn tests_take_their_longest_or_their_total_over_the_processors() {
        let seconds = [1.0, 2.0, 8.0, 3.0];
        assert_eq!(wall(&set(&[0, 1, 3]), &seconds, 2), 3.0);
        assert_eq!(wall(&set(&[0, 1, 3]), &seconds, 1), 6.0);
        assert_eq!(wall(&set(&[1, 2]), &seconds, 2), 8.0);
        assert_eq!(wall(&set(&[0, 1, 2, 3]), &seconds, 0), 14.0);
        assert_eq!(wall(&set(&[]), &seconds, 4), 0.0);
    }

    #[test]
    fn mutants_with_the_same_tests_are_tested_together_and_uncovered_ones_not_at_all() {
        let seconds = [10.0, 20.0];
        let mutants = [
            (mutant("a", "x.rs", 1, 1), set(&[0])),
            (mutant("b", "y.rs", 1, 1), set(&[])),
            (mutant("c", "y.rs", 2, 2), set(&[0, 1])),
            (mutant("d", "z.rs", 3, 3), set(&[0])),
            (mutant("e", "x.rs", 7, 9), set(&[0])),
        ];
        let (chunks, uncovered) = chunks(&mutants, &seconds, 1);
        assert_eq!(uncovered, ["b"]);
        assert_eq!(
            chunks,
            [
                Chunk {
                    mutants: vec!["a".into(), "d".into(), "e".into()],
                    lines: marked(&[("x.rs", &[1, 7][..]), ("z.rs", &[3][..])]),
                    tests: set(&[0]),
                    estimate: 3.0 * (BUILD_SECONDS + 10.0),
                    shard: 0,
                },
                Chunk {
                    mutants: vec!["c".into()],
                    lines: marked(&[("y.rs", &[2][..])]),
                    tests: set(&[0, 1]),
                    estimate: BUILD_SECONDS + 30.0,
                    shard: 0,
                },
            ]
        );
    }

    #[test]
    fn a_long_group_is_split_into_chunks_a_shard_can_stop_between() {
        // Each mutant takes BUILD_SECONDS + 280 s = 300 s: three fit in a chunk.
        let seconds = [280.0];
        let mutants: Vec<_> = (0..7)
            .map(|at| (mutant(&at.to_string(), "x.rs", at, at), set(&[0])))
            .collect();
        let (split, _) = chunks(&mutants, &seconds, 1);
        let sizes: Vec<usize> = split.iter().map(|chunk| chunk.mutants.len()).collect();
        assert_eq!(sizes, [3, 3, 1]);
        assert_eq!(split[2].estimate, 300.0);
        // A mutant longer than a chunk is a chunk of its own.
        let (alone, _) = chunks(&mutants[..2], &[2000.0], 1);
        assert_eq!(alone.len(), 2);
    }

    #[test]
    fn shards_hold_four_hours_each_up_to_the_limit() {
        assert_eq!(shard_count(0.0), 0);
        assert_eq!(shard_count(1.0), 1);
        assert_eq!(shard_count(SHARD_SECONDS), 1);
        assert_eq!(shard_count(SHARD_SECONDS + 1.0), 2);
        assert_eq!(shard_count(40.0 * 3600.0), 10);
        assert_eq!(shard_count(1000.0 * 3600.0), MAX_SHARDS);
    }

    /// A shard's planned work fits before its deadline, and the chunk running at the deadline,
    /// even at twice its estimate, ends within the job's six hours.
    #[test]
    fn a_shard_stops_late_enough_for_its_plan_and_early_enough_to_report() {
        let deadline = SHARD_DEADLINE.as_secs_f64();
        let job = 6.0 * 3600.0;
        assert!((SHARD_SECONDS..job - 2.0 * CHUNK_SECONDS).contains(&deadline));
    }

    fn chunk(estimate: f64) -> Chunk {
        Chunk {
            mutants: Vec::new(),
            lines: BTreeMap::new(),
            tests: BTreeSet::new(),
            estimate,
            shard: 9,
        }
    }

    #[test]
    fn the_longest_chunks_go_first_to_the_least_loaded_shard() {
        // 5 → 0, 4 → 1, 3 → 1 (7), 2 → 0 (7), 1 → 0, the first of two equal: 8 against 7.
        let mut chunks: Vec<Chunk> = [1.0, 5.0, 3.0, 4.0, 2.0].map(chunk).to_vec();
        assert_shards(&mut chunks, 2, &[0, 0, 1, 1, 0]);
        let mut chunks: Vec<Chunk> = [5.0, 1.0, 1.0, 1.0].map(chunk).to_vec();
        assert_shards(&mut chunks, 3, &[0, 1, 2, 1]);
        let mut chunks: Vec<Chunk> = [5.0, 1.0].map(chunk).to_vec();
        assert_shards(&mut chunks, 0, &[0, 0]);
    }

    fn assert_shards(chunks: &mut [Chunk], shards: usize, expected: &[usize]) {
        assign(chunks, shards);
        let got: Vec<usize> = chunks.iter().map(|chunk| chunk.shard).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn the_plan_names_each_chunks_tests_and_shard_and_is_read_back() {
        let tests = ["t0".to_string(), "t1".to_string()];
        let chunks = vec![
            Chunk {
                mutants: vec!["a".into(), "b".into()],
                lines: marked(&[("x.rs", &[4, 9][..])]),
                tests: set(&[1]),
                estimate: 40.4,
                shard: 0,
            },
            Chunk {
                mutants: vec!["c".into()],
                lines: marked(&[("y.rs", &[2][..]), ("z.rs", &[1][..])]),
                tests: set(&[0, 1]),
                estimate: 30.0,
                shard: 1,
            },
        ];
        let plan = plan_json("abc", 2, &chunks, &["u".to_string()], &tests);
        assert_eq!(plan["base"], "abc");
        assert_eq!(plan["shards"], 2);
        let (first, uncovered) = planned_for(&plan, 0).unwrap();
        assert_eq!(
            first,
            [Planned {
                estimate: 40.0,
                lines: BTreeMap::from([("x.rs".to_string(), vec![4, 9])]),
                mutants: vec!["a".into(), "b".into()],
                tests: vec!["t1".into()],
            }]
        );
        assert_eq!(uncovered, ["u"]);
        let (second, uncovered) = planned_for(&plan, 1).unwrap();
        assert_eq!(second[0].tests, ["t0", "t1"]);
        assert_eq!(second[0].mutants, ["c"]);
        assert_eq!(
            second[0].lines,
            BTreeMap::from([("y.rs".to_string(), vec![2]), ("z.rs".to_string(), vec![1])])
        );
        assert!(uncovered.is_empty());
        assert!(planned_for(&plan, 2).unwrap().0.is_empty());
        assert!(planned_for(&json!({}), 0).is_err());
    }

    #[test]
    fn the_matrix_lists_each_shard() {
        assert_eq!(outputs(0, 0), "count=0\nshards=[]\n");
        assert_eq!(outputs(12, 3), "count=12\nshards=[0,1,2]\n");
    }

    #[test]
    fn a_mutant_is_chosen_by_its_exact_name() {
        assert_eq!(
            exact_re("a.rs:3:5: replace f -> Option<(u8, &str)> with None"),
            r"^a\.rs:3:5: replace f \-> Option<\(u8, \&str\)> with None$"
        );
        assert_eq!(exact_re("[]{}^$#~|*+?\\"), r"^\[\]\{\}\^\$\#\~\|\*\+\?\\$");
    }

    #[test]
    fn a_chunks_diff_marks_its_lines_alone_as_added() {
        let sources = |file: &str| -> Result<String> {
            match file {
                "a.rs" => Ok("one\ntwo\nthree\nfour\nfive\nsix\n".into()),
                "b.rs" => Ok("x\ny\n".into()),
                _ => Err(format!("{file}: missing")),
            }
        };
        let lines = BTreeMap::from([
            ("a.rs".to_string(), vec![5, 1, 2, 5]),
            ("b.rs".to_string(), vec![2]),
        ]);
        // Lines 1-2 then 5: the old file lacks lines 1 and 2, so line 5 comes after its old 2.
        assert_eq!(
            chunk_diff(&lines, sources).unwrap(),
            "--- a/a.rs\n+++ b/a.rs\n@@ -0,0 +1,2 @@\n+one\n+two\n@@ -2,0 +5,1 @@\n+five\n\
             --- a/b.rs\n+++ b/b.rs\n@@ -1,0 +2,1 @@\n+y\n"
        );
        let past_the_end = BTreeMap::from([("b.rs".to_string(), vec![3])]);
        assert!(chunk_diff(&past_the_end, sources).is_err());
        let missing = BTreeMap::from([("c.rs".to_string(), vec![1])]);
        assert!(chunk_diff(&missing, sources).is_err());
    }

    #[test]
    fn a_chunk_runs_cargo_mutants_on_its_mutants_with_its_tests() {
        let chunk = Planned {
            estimate: 1.0,
            lines: BTreeMap::from([
                ("apps/gpui/src/a.rs".to_string(), vec![3]),
                ("apps/gpui/src/b.rs".to_string(), vec![1]),
            ]),
            mutants: vec!["m.1".into(), "m2".into()],
            tests: vec!["e2e::x".into(), "a::tests::y".into()],
        };
        let args = chunk_args(&chunk, Path::new("out/3"), Path::new("out/3/chunk.patch"));
        let expected: Vec<String> = [
            "mutants",
            "--in-place",
            "--baseline",
            "skip",
            "--no-shuffle",
            "--test-tool",
            "nextest",
            "--package",
            APP,
            "--timeout",
            "300",
            "--build-timeout",
            "600",
            "--output",
            "out/3",
            "--in-diff",
            "out/3/chunk.patch",
            "--file",
            "apps/gpui/src/a.rs",
            "--file",
            "apps/gpui/src/b.rs",
            "--re",
            r"^m\.1$",
            "--re",
            "^m2$",
            "--cargo-test-arg",
            "-E",
            "--cargo-test-arg",
            "test(=e2e::x) | test(=a::tests::y)",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(args, expected);
    }

    #[test]
    fn a_shard_adds_up_its_chunks_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let (one, two, top) = (
            dir.path().join("1"),
            dir.path().join("2"),
            dir.path().join("top"),
        );
        for path in [&one, &two, &top] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::write(one.join("missed.txt"), "a\n").unwrap();
        std::fs::write(one.join("caught.txt"), "b\nc\n").unwrap();
        std::fs::write(two.join("missed.txt"), "d\n").unwrap();
        std::fs::write(two.join("timeout.txt"), "e\n").unwrap();
        std::fs::write(two.join("unviable.txt"), "f\n").unwrap();
        std::fs::write(two.join("other.txt"), "g\n").unwrap();
        merge(&one, &top).unwrap();
        merge(&two, &top).unwrap();
        assert_eq!(
            std::fs::read_to_string(top.join("missed.txt")).unwrap(),
            "a\nd\n"
        );
        assert_eq!(count_in(&top, "missed"), 2);
        assert_eq!(count_in(&top, "caught"), 2);
        assert_eq!(count_in(&top, "timeout"), 1);
        assert_eq!(count_in(&top, "unviable"), 1);
        assert_eq!(count_in(&top, "other"), 0);
        assert!(merge(&one, &dir.path().join("none/there")).is_err());
    }

    #[test]
    fn a_shard_passes_only_when_nothing_survived_or_was_left() {
        assert!(verdict(0, 0, 0, 0).is_ok());
        for counts in [(1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1)] {
            let error = verdict(counts.0, counts.1, counts.2, counts.3).unwrap_err();
            assert!(error.starts_with(&format!(
                "{} missed, {} timed out, {} on lines no test runs, {} not reached",
                counts.0, counts.1, counts.2, counts.3
            )));
        }
    }

    #[test]
    fn lists_are_written_a_line_each() {
        assert_eq!(lines(&["a".into(), "b".into()]), "a\nb\n");
        assert_eq!(lines(&[]), "");
    }

    #[test]
    fn work_is_done_for_each_item_in_order() {
        let results = in_parallel(5, 3, |at| at as f64 * 2.0);
        assert_eq!(results, [0.0, 2.0, 4.0, 6.0, 8.0]);
        assert_eq!(in_parallel(2, 0, |at| at as f64), [0.0, 1.0]);
    }
}
