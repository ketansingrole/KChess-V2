//! `cargo xtask ci-times [--days <n>]`: how long CI took over the last n days (7), job by job, and
//! the tests that failed and then passed on the same commit. The weekly CI times workflow writes
//! it to its run's summary, so a slower job or a new flaky test shows within a week.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::util::{Result, output, repository, say};

/// Asks GitHub through `gh` with these arguments; the tests answer it themselves.
type Gh<'a> = &'a dyn Fn(&[&str]) -> Result<String>;

/// The completed CI runs, one per line: id, event, commit, conclusion, created (Unix seconds).
const RUNS: &str = r#".workflow_runs[] | select(.status == "completed") | [.id, .event, .head_sha, .conclusion, (.created_at | fromdate)] | @tsv"#;

/// A run's jobs over all its attempts, one per line: id, attempt, name, conclusion, started and
/// completed (Unix seconds).
const JOBS: &str = r#".jobs[] | select(.started_at and .completed_at) | [.id, .run_attempt, .name, .conclusion, (.started_at | fromdate), (.completed_at | fromdate)] | @tsv"#;

pub fn run(args: &[String]) -> Result {
    let days = days(args)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|cause| cause.to_string())?
        .as_secs();
    print!("{}", report_for(days, now, &|args| output("gh", args))?);
    Ok(())
}

fn days(args: &[String]) -> Result<u64> {
    match args {
        [] => Ok(7),
        [flag, n] if flag == "--days" => n
            .parse()
            .ok()
            .filter(|days| *days > 0)
            .ok_or_else(|| format!("--days needs a number of days, not {n}.")),
        _ => Err("ci-times takes only --days <n>.".into()),
    }
}

/// The report of the `days` before `now` (Unix seconds), from what `gh` answers.
fn report_for(days: u64, now: u64, gh: Gh) -> Result<String> {
    let since = date(now.saturating_sub(days * 86_400));
    let repo = repository();
    say(&format!("Reading CI's runs since {since}…"));
    let runs = parse_runs(&gh(&[
        "api",
        "--paginate",
        &format!("repos/{repo}/actions/workflows/ci.yml/runs?per_page=100&created=>={since}"),
        "--jq",
        RUNS,
    ])?);
    let mut jobs = Vec::new();
    for run in &runs {
        jobs.extend(parse_jobs(
            run.id,
            &gh(&[
                "api",
                "--paginate",
                &format!(
                    "repos/{repo}/actions/runs/{}/jobs?per_page=100&filter=all",
                    run.id
                ),
                "--jq",
                JOBS,
            ])?,
        ));
    }
    let mut flaky = Vec::new();
    for job in flaky_failures(&runs, &jobs) {
        // A log GitHub no longer keeps names no test; the job still counts.
        let log = gh(&[
            "api",
            "--allow-escape-sequences",
            &format!("repos/{repo}/actions/jobs/{}/logs", job.id),
        ])
        .unwrap_or_default();
        flaky.push((job, failed_tests(&log)));
    }
    Ok(report(days, &since, &repo, &runs, &jobs, &flaky))
}

#[derive(Debug, Clone, PartialEq)]
struct Run {
    id: u64,
    event: String,
    sha: String,
    conclusion: String,
    created: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct Job {
    id: u64,
    run: u64,
    attempt: u32,
    name: String,
    conclusion: String,
    started: u64,
    completed: u64,
}

impl Job {
    fn minutes(&self) -> f64 {
        self.completed.saturating_sub(self.started) as f64 / 60.0
    }

    /// Ran to a result: neither skipped nor cancelled.
    fn finished(&self) -> bool {
        matches!(self.conclusion.as_str(), "success" | "failure")
    }
}

fn parse_runs(tsv: &str) -> Vec<Run> {
    tsv.lines()
        .filter_map(|line| match line.split('\t').collect::<Vec<_>>()[..] {
            [id, event, sha, conclusion, created] => Some(Run {
                id: id.parse().ok()?,
                event: event.into(),
                sha: sha.into(),
                conclusion: conclusion.into(),
                created: created.parse().ok()?,
            }),
            _ => None,
        })
        .collect()
}

fn parse_jobs(run: u64, tsv: &str) -> Vec<Job> {
    tsv.lines()
        .filter_map(|line| match line.split('\t').collect::<Vec<_>>()[..] {
            [id, attempt, name, conclusion, started, completed] => Some(Job {
                id: id.parse().ok()?,
                run,
                attempt: attempt.parse().ok()?,
                name: name.into(),
                conclusion: conclusion.into(),
                started: started.parse().ok()?,
                completed: completed.parse().ok()?,
            }),
            _ => None,
        })
        .collect()
}

/// A matrix job under its job's name and platform: `mutants (3)` is `mutants`, and
/// `headless (windows-latest, -p kchess-cli …)` is `headless (windows-latest)`.
fn normalize(name: &str) -> String {
    let Some((job, rest)) = name.split_once(" (") else {
        return name.to_string();
    };
    let inner = rest.strip_suffix(')').unwrap_or(rest);
    if inner.chars().all(|c| c.is_ascii_digit()) {
        return job.to_string();
    }
    let platform = inner.split(", ").next().unwrap_or(inner);
    format!("{job} ({platform})")
}

/// The jobs that failed on a commit where the same job, by its full name, also passed: in a
/// later attempt of the run, or in another run of that commit.
fn flaky_failures(runs: &[Run], jobs: &[Job]) -> Vec<Job> {
    let sha: BTreeMap<u64, &str> = runs.iter().map(|run| (run.id, run.sha.as_str())).collect();
    let mut by_job: BTreeMap<(&str, &str), (bool, Vec<&Job>)> = BTreeMap::new();
    // `ci-passed` fails whenever another job does, and passes once they all do.
    for job in jobs
        .iter()
        .filter(|job| job.finished() && job.name != "ci-passed")
    {
        let Some(commit) = sha.get(&job.run) else {
            continue;
        };
        let entry = by_job.entry((commit, job.name.as_str())).or_default();
        if job.conclusion == "success" {
            entry.0 = true;
        } else {
            entry.1.push(job);
        }
    }
    by_job
        .into_values()
        .filter(|(passed, _)| *passed)
        .flat_map(|(_, failed)| failed.into_iter().cloned())
        .collect()
}

/// The tests a job's log reports failed, each once: nextest's `FAIL [` and `TIMEOUT [` lines, or
/// where nextest did not run, `cargo test`'s `test … ... FAILED`. nextest's output also carries the
/// failed test's own `cargo test` lines, under a shorter name.
fn failed_tests(log: &str) -> Vec<String> {
    let mut nextest: Vec<String> = Vec::new();
    let mut libtest: Vec<String> = Vec::new();
    for line in log.lines().map(strip_escapes) {
        let (tests, name) = if let Some(at) = line.find("FAIL [").or_else(|| line.find("TIMEOUT ["))
        {
            let name = line[at..].split_once("] ").map(|(_, rest)| {
                let rest = rest.trim();
                // nextest numbers each test: `( 687/1109) crate::binary name`.
                match rest.strip_prefix('(') {
                    Some(numbered) => numbered.split_once(") ").map_or(rest, |(_, name)| name),
                    None => rest,
                }
                .trim()
                .to_string()
            });
            (&mut nextest, name)
        } else {
            let name = line
                .trim_end()
                .strip_suffix(" ... FAILED")
                .and_then(|head| head.split_once("test ").map(|(_, name)| name.to_string()));
            (&mut libtest, name)
        };
        if let Some(name) = name.filter(|name| !name.is_empty())
            && !tests.contains(&name)
        {
            tests.push(name);
        }
    }
    if nextest.is_empty() { libtest } else { nextest }
}

/// A log line without its terminal colours (`ESC [ … m`).
fn strip_escapes(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            plain.push(c);
        }
    }
    plain
}

/// The value below which `share` of the `sorted` values fall (nearest rank).
fn percentile(sorted: &[f64], share: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (share * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn row(label: &str, mut minutes: Vec<f64>, extra: &str) -> String {
    minutes.sort_by(f64::total_cmp);
    format!(
        "| {label} | {}{extra} | {:.1} min | {:.1} min | {:.1} min |\n",
        minutes.len(),
        percentile(&minutes, 0.5),
        percentile(&minutes, 0.9),
        minutes.last().copied().unwrap_or(0.0),
    )
}

/// The Markdown report.
fn report(
    days: u64,
    since: &str,
    repo: &str,
    runs: &[Run],
    jobs: &[Job],
    flaky: &[(Job, Vec<String>)],
) -> String {
    let mut out = format!("## CI times: the last {days} days (since {since})\n\n");
    // A run's own time: from its creation to its first attempt's last job, for the runs no newer
    // push cancelled.
    out += "| Runs | Count | Median | 90% within | Slowest |\n|---|---|---|---|---|\n";
    for (event, label) in [
        ("pull_request", "Pull requests"),
        ("push", "Pushes to main"),
        ("schedule", "Nightly"),
    ] {
        let minutes: Vec<f64> = runs
            .iter()
            .filter(|run| run.event == event && run.conclusion != "cancelled")
            .filter_map(|run| {
                let first: Vec<&Job> = jobs
                    .iter()
                    .filter(|job| job.run == run.id && job.attempt == 1)
                    .collect();
                if first.is_empty() || first.iter().any(|job| job.conclusion == "cancelled") {
                    return None;
                }
                let end = first.iter().map(|job| job.completed).max()?;
                Some(end.saturating_sub(run.created) as f64 / 60.0)
            })
            .collect();
        if !minutes.is_empty() {
            out += &row(label, minutes, "");
        }
    }
    let mut by_name: BTreeMap<String, (Vec<f64>, usize)> = BTreeMap::new();
    for job in jobs.iter().filter(|job| job.finished()) {
        let entry = by_name.entry(normalize(&job.name)).or_default();
        entry.0.push(job.minutes());
        entry.1 += usize::from(job.conclusion == "failure");
    }
    let mut names: Vec<(String, (Vec<f64>, usize))> = by_name.into_iter().collect();
    names.sort_by(|a, b| {
        let p90 = |minutes: &[f64]| {
            let mut sorted = minutes.to_vec();
            sorted.sort_by(f64::total_cmp);
            percentile(&sorted, 0.9)
        };
        p90(&b.1.0).total_cmp(&p90(&a.1.0))
    });
    out += "\n### Jobs, slowest first\n\n| Job | Runs | Median | 90% within | Slowest |\n|---|---|---|---|---|\n";
    for (name, (minutes, failed)) in names {
        let extra = if failed > 0 {
            format!(" ({failed} failed)")
        } else {
            String::new()
        };
        out += &row(&name, minutes, &extra);
    }
    out += "\n### Flaky tests\n\n";
    if flaky.is_empty() {
        out += "None: no job failed and then passed on the same commit.\n";
        return out;
    }
    let mut tests: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    let mut unnamed: BTreeMap<String, usize> = BTreeMap::new();
    for (job, names) in flaky {
        if names.is_empty() {
            *unnamed.entry(normalize(&job.name)).or_default() += 1;
        }
        for name in names {
            let entry = tests.entry(name).or_insert((0, job.run));
            entry.0 += 1;
            entry.1 = entry.1.max(job.run);
        }
    }
    out += "Failed, then passed on the same commit.\n\n";
    if !tests.is_empty() {
        let mut tests: Vec<_> = tests.into_iter().collect();
        tests.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
        out += "| Test | Times | Last failed in |\n|---|---|---|\n";
        for (name, (times, run)) in tests {
            out += &format!(
                "| `{name}` | {times} | [run {run}](https://github.com/{repo}/actions/runs/{run}) |\n"
            );
        }
    }
    if !unnamed.is_empty() {
        out += "\nJobs that failed, then passed, with no failed test in their log:";
        for (name, times) in unnamed {
            out += &format!(" {name} ×{times};");
        }
        out.pop();
        out += ".\n";
    }
    out
}

/// `YYYY-MM-DD` of a Unix time in seconds (UTC).
fn date(seconds: u64) -> String {
    // Howard Hinnant's days-to-civil, for days since 1970-01-01.
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn job(id: u64, run: u64, attempt: u32, name: &str, conclusion: &str, minutes: u64) -> Job {
        Job {
            id,
            run,
            attempt,
            name: name.into(),
            conclusion: conclusion.into(),
            started: 1_000,
            completed: 1_000 + minutes * 60,
        }
    }

    fn run(id: u64, event: &str, sha: &str) -> Run {
        Run {
            id,
            event: event.into(),
            sha: sha.into(),
            conclusion: "success".into(),
            created: 1_000,
        }
    }

    #[test]
    fn days_default_to_a_week_and_refuse_anything_else() {
        assert_eq!(days(&[]), Ok(7));
        assert_eq!(days(&["--days".into(), "30".into()]), Ok(30));
        assert!(days(&["--days".into(), "0".into()]).is_err());
        assert!(days(&["--days".into(), "week".into()]).is_err());
        assert!(days(&["--weeks".into(), "2".into()]).is_err());
    }

    #[test]
    fn dates_are_utc_days() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_793_750_400 + 86_399), "2026-11-04");
    }

    #[test]
    fn matrix_jobs_count_as_their_job_and_platform() {
        assert_eq!(normalize("mutants (3)"), "mutants");
        assert_eq!(
            normalize("headless (windows-latest, -p kchess-cli -p xtask)"),
            "headless (windows-latest)"
        );
        assert_eq!(normalize("gpui (macos-latest)"), "gpui (macos-latest)");
        assert_eq!(normalize("snap / build"), "snap / build");
    }

    #[test]
    fn percentiles_take_the_nearest_rank() {
        let minutes = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert_eq!(percentile(&minutes, 0.5), 5.0);
        assert_eq!(percentile(&minutes, 0.9), 9.0);
        assert_eq!(percentile(&minutes, 1.0), 10.0);
        assert_eq!(percentile(&[4.0], 0.9), 4.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }

    #[test]
    fn runs_and_jobs_are_read_from_the_tsv_lines() {
        assert_eq!(
            parse_runs("7\tpull_request\tabc\tsuccess\t100\nbroken line\n"),
            [Run {
                id: 7,
                event: "pull_request".into(),
                sha: "abc".into(),
                conclusion: "success".into(),
                created: 100,
            }]
        );
        assert_eq!(
            parse_jobs(7, "9\t2\tcheck\tfailure\t100\t160\n9\tx\tcheck\n"),
            [Job {
                id: 9,
                run: 7,
                attempt: 2,
                name: "check".into(),
                conclusion: "failure".into(),
                started: 100,
                completed: 160,
            }]
        );
    }

    #[test]
    fn a_job_is_flaky_when_it_failed_and_passed_on_one_commit() {
        let runs = [
            run(1, "pull_request", "a"),
            run(2, "pull_request", "a"),
            run(3, "pull_request", "b"),
        ];
        let jobs = [
            // Failed, then passed when the run was re-run: flaky.
            job(10, 1, 1, "gpui (windows-latest)", "failure", 9),
            job(11, 1, 2, "gpui (windows-latest)", "success", 9),
            // Failed in one run of the commit and passed in another: flaky.
            job(12, 1, 1, "check", "failure", 5),
            job(13, 2, 1, "check", "success", 5),
            // Another shard passing proves nothing about this one.
            job(14, 1, 1, "mutants (0)", "failure", 3),
            job(15, 1, 1, "mutants (1)", "success", 3),
            // Failed on another commit only: a real failure.
            job(16, 3, 1, "gpui (windows-latest)", "failure", 9),
            // `ci-passed` only follows the other jobs.
            job(19, 1, 1, "ci-passed", "failure", 0),
            job(20, 1, 2, "ci-passed", "success", 0),
            // Cancelled is no result.
            job(17, 3, 1, "check", "cancelled", 1),
            job(18, 3, 2, "check", "success", 5),
        ];
        let ids: Vec<u64> = flaky_failures(&runs, &jobs)
            .iter()
            .map(|job| job.id)
            .collect();
        assert_eq!(ids, [12, 10]);
    }

    #[test]
    fn failed_tests_are_named_once_from_nextest_and_cargo_test_logs() {
        let log = "\
2026-11-10T00:00:40Z \u{1b}[31;1m        FAIL\u{1b}[0m [   4.500s] ( 687/1109) kchess-core::explorer_shutdown close_cancels
2026-11-10T00:01:01Z         FAIL [   4.500s] ( 687/1109) kchess-core::explorer_shutdown close_cancels
2026-11-10T00:01:01Z     test close_cancels ... FAILED
2026-11-10T00:01:02Z      TIMEOUT [  60.001s] kchess-cli::cli hangs
";
        assert_eq!(
            failed_tests(log),
            [
                "kchess-core::explorer_shutdown close_cancels",
                "kchess-cli::cli hangs"
            ]
        );
        let log = "\
2026-11-10T00:01:03Z test e2e::practice::a_game_opened_for_guessing ... FAILED
2026-11-10T00:01:04Z test e2e::practice::passes ... ok
2026-11-10T00:01:05Z test e2e::practice::a_game_opened_for_guessing ... FAILED
2026-11-10T00:01:06Z test result: FAILED. 1 passed; 1 failed
";
        assert_eq!(
            failed_tests(log),
            ["e2e::practice::a_game_opened_for_guessing"]
        );
        assert!(failed_tests("FAIL [ 1s]").is_empty());
    }

    #[test]
    fn the_report_ranks_jobs_and_names_flaky_tests() {
        let runs = [
            Run {
                created: 0,
                ..run(1, "pull_request", "a")
            },
            Run {
                created: 0,
                ..run(2, "push", "b")
            },
            Run {
                created: 0,
                conclusion: "cancelled".into(),
                ..run(3, "pull_request", "c")
            },
        ];
        let jobs = [
            job(10, 1, 1, "gpui (windows-latest)", "failure", 9),
            job(11, 1, 2, "gpui (windows-latest)", "success", 8),
            job(12, 1, 1, "check", "success", 5),
            job(13, 2, 1, "check", "success", 6),
            job(14, 1, 1, "snap", "skipped", 0),
        ];
        let flaky = [(jobs[0].clone(), vec!["e2e::practice::slow".to_string()])];
        let text = report(7, "2026-11-03", "o/r", &runs, &jobs, &flaky);
        assert!(
            text.starts_with("## CI times: the last 7 days (since 2026-11-03)\n"),
            "{text}"
        );
        // From the run's creation (0 s) to its first attempt's last job (1000 s + 9 min).
        assert!(
            text.contains("| Pull requests | 1 | 25.7 min | 25.7 min | 25.7 min |"),
            "{text}"
        );
        assert!(text.contains("| Pushes to main | 1 | 22.7 min |"), "{text}");
        assert!(!text.contains("Nightly"), "{text}");
        let gpui =
            text.find("| gpui (windows-latest) | 2 (1 failed) | 8.0 min | 9.0 min | 9.0 min |");
        let check = text.find("| check | 2 | 5.0 min | 6.0 min | 6.0 min |");
        assert!(gpui.is_some() && check.is_some() && gpui < check, "{text}");
        assert!(!text.contains("| snap |"), "{text}");
        assert!(
            text.contains(
                "| `e2e::practice::slow` | 1 | [run 1](https://github.com/o/r/actions/runs/1) |"
            ),
            "{text}"
        );
        assert!(!text.contains("no failed test in their log"), "{text}");
    }

    #[test]
    fn a_flaky_job_without_a_named_test_is_listed_by_job() {
        let jobs = [job(10, 1, 1, "mutants (2)", "failure", 9)];
        let flaky = [(jobs[0].clone(), Vec::new()), (jobs[0].clone(), Vec::new())];
        let text = report(7, "d", "o/r", &[run(1, "pull_request", "a")], &jobs, &flaky);
        assert!(
            text.ends_with(
                "Jobs that failed, then passed, with no failed test in their log: mutants ×2.\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_quiet_week_says_so() {
        let text = report(7, "d", "o/r", &[], &[], &[]);
        assert!(
            text.ends_with(
                "### Flaky tests\n\nNone: no job failed and then passed on the same commit.\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn the_report_asks_github_for_each_run_and_each_flaky_jobs_log() {
        let asked = RefCell::new(Vec::new());
        let gh = |args: &[&str]| -> Result<String> {
            let path = args
                .iter()
                .find(|arg| arg.starts_with("repos/"))
                .unwrap()
                .to_string();
            asked.borrow_mut().push(path.clone());
            Ok(if path.contains("/workflows/ci.yml/runs") {
                "1\tpull_request\ta\tsuccess\t0\n".into()
            } else if path.ends_with("/jobs?per_page=100&filter=all") {
                "10\t1\tcheck\tfailure\t0\t60\n11\t2\tcheck\tsuccess\t0\t60\n".into()
            } else {
                "test flaky::one ... FAILED\n".into()
            })
        };
        // 2026-11-10, a week back from it.
        let text = report_for(7, 1_794_268_800, &gh).unwrap();
        let repo = repository();
        assert_eq!(
            *asked.borrow(),
            [
                format!(
                    "repos/{repo}/actions/workflows/ci.yml/runs?per_page=100&created=>=2026-11-03"
                ),
                format!("repos/{repo}/actions/runs/1/jobs?per_page=100&filter=all"),
                format!("repos/{repo}/actions/jobs/10/logs"),
            ]
        );
        assert!(text.contains("| `flaky::one` | 1 |"), "{text}");
    }

    #[test]
    fn github_failing_stops_the_report() {
        let gh = |_: &[&str]| -> Result<String> { Err("gh: not logged in".into()) };
        assert_eq!(report_for(7, 0, &gh), Err("gh: not logged in".into()));
    }
}
