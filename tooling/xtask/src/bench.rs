//! `bench`: the GPUI app's frame benchmarks (`apps/gpui/src/bench.rs`) for one or more builds.
//! Each run gets a fresh throwaway profile; the builds alternate run by run (odd runs reverse the
//! order), so a change in the machine's load falls on all of them; the report gives medians.
//!
//! A build is the binary of `cargo build --release -p kchess-gpui --features bench`
//! (`target/release/KChess`), copied aside, since the next build replaces it. Nothing else should
//! run while measuring. A run whose window was covered for a while (`visible_fraction` below 1)
//! draws no frames then; it is reported and left out.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::util::{Result, say};

const SCENARIOS: [&str; 10] = [
    "home-idle",
    "local-clock",
    "board-animation",
    "analysis-engine",
    "analysis-stepping",
    "openings-shell",
    "lichess-idle",
    "lichess-game",
    "lichess-tv",
    "pages-tour",
];

/// The figures compared, as the app reports them.
const METRICS: [&str; 12] = [
    "draws_per_s",
    "latency_ms_p50",
    "latency_ms_p95",
    "draw_ms_p50",
    "draw_ms_p95",
    "draw_ms_per_s",
    "main_thread_busy_fraction",
    "process_cpu_fraction",
    "max_rss_mib",
    "journal_entries_lost",
    "page_open_ms_p50",
    "page_open_ms_p95",
];

struct Options {
    builds: Vec<(String, PathBuf)>,
    runs: usize,
    seconds: u64,
    scenarios: Vec<String>,
    out: Option<PathBuf>,
}

fn options(args: &[String]) -> Result<Options> {
    let mut options = Options {
        builds: Vec::new(),
        runs: 5,
        seconds: 30,
        scenarios: SCENARIOS.iter().map(|name| name.to_string()).collect(),
        out: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value."))
        };
        match arg.as_str() {
            "--build" => {
                let build = value()?;
                let (label, path) = build.split_once('=').ok_or("--build is label=binary.")?;
                options
                    .builds
                    .push((label.to_string(), PathBuf::from(path)));
            }
            "--runs" => options.runs = value()?.parse().map_err(|_| "--runs is a count.")?,
            "--seconds" => {
                options.seconds = value()?.parse().map_err(|_| "--seconds is a count.")?
            }
            "--scenarios" => {
                options.scenarios = value()?.split(',').map(str::to_string).collect();
            }
            "--out" => options.out = Some(PathBuf::from(value()?)),
            other => return Err(format!("Unknown bench option {other}.")),
        }
    }
    if options.builds.is_empty() {
        return Err("Name at least one --build label=binary.".into());
    }
    Ok(options)
}

/// `cargo xtask bench --build <label>=<binary> [--build …] [--runs 5] [--seconds 30]
/// [--scenarios a,b] [--out results.json]`
pub fn run(args: &[String]) -> Result {
    let options = options(args)?;
    let engine = crate::app::engine()?;
    let mut results: Vec<Value> = Vec::new();
    for run in 1..=options.runs {
        for scenario in &options.scenarios {
            let mut builds: Vec<&(String, PathBuf)> = options.builds.iter().collect();
            if run % 2 == 1 {
                builds.reverse();
            }
            for (label, binary) in builds {
                say(&format!("run {run}: {scenario} on {label}"));
                let report = measure(binary, scenario, options.seconds, &engine)?;
                if report["visible_fraction"].as_f64().unwrap_or(1.) < 1. {
                    say(&format!(
                        "  left out: the window was covered ({label}, run {run})"
                    ));
                    continue;
                }
                say(&format!(
                    "  ended on {} as {}, {} moves in the online game",
                    report["page"], report["account"], report["online_moves"]
                ));
                results.push(json!({ "build": label, "run": run, "report": report }));
            }
        }
    }
    if let Some(out) = &options.out {
        let text = serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?;
        std::fs::write(out, text).map_err(|e| format!("Could not write {}: {e}", out.display()))?;
    }
    print!("{}", summary(&options, &results));
    Ok(())
}

/// One run of one scenario, on a profile of its own. The `lichess-*` scenarios sign in to a
/// stand-in Lichess (`fake_lichess`), through the core's test origin.
fn measure(binary: &Path, scenario: &str, seconds: u64, engine: &Path) -> Result<Value> {
    let profile = tempfile::tempdir().map_err(|e| format!("No temporary profile: {e}"))?;
    let out = profile.path().join("bench.json");
    let lichess = crate::fake_lichess::FakeLichess::start(scenario)?;
    let app = |bench: &str| {
        let mut command = Command::new(binary);
        if let Some(lichess) = &lichess {
            command
                .env("KCHESS_STORE_DEBUG", "1")
                .env("KCHESS_TEST_LICHESS_BASE", &lichess.base);
        }
        command
            .env("KCHESS_USER_DATA_DIR", profile.path().join("profile"))
            .env("KCHESS_BENCH", bench);
        command
    };
    if lichess.is_some() {
        // Signed in by an earlier launch, as someone who plays on Lichess is.
        app("lichess-login")
            .env("KCHESS_ENGINE_PATH", engine)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("Could not start {}: {e}", binary.display()))?;
    }
    let status = app(scenario)
        .env("KCHESS_ENGINE_PATH", engine)
        .env("KCHESS_BENCH_SECONDS", seconds.to_string())
        .env("KCHESS_BENCH_OUT", &out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Could not start {}: {e}", binary.display()))?;
    if let Some(lichess) = &lichess {
        for path in lichess.unknown() {
            say(&format!("  Lichess stand-in has no answer for {path}"));
        }
    }
    let text = std::fs::read_to_string(&out).map_err(|e| {
        format!(
            "{} wrote no report for {scenario} ({status}): {e}",
            binary.display()
        )
    })?;
    serde_json::from_str(&text).map_err(|e| format!("An unreadable report for {scenario}: {e}"))
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        n if n % 2 == 1 => Some(values[middle]),
        _ => Some((values[middle - 1] + values[middle]) / 2.),
    }
}

/// Each scenario's medians by build, and the first build's over the last's.
fn summary(options: &Options, results: &[Value]) -> String {
    let mut text = String::new();
    for scenario in &options.scenarios {
        text.push_str(&format!("\n{scenario}\n"));
        for metric in METRICS {
            let medians: Vec<Option<f64>> = options
                .builds
                .iter()
                .map(|(label, _)| {
                    median(
                        results
                            .iter()
                            .filter(|result| {
                                result["build"] == label.as_str()
                                    && result["report"]["scenario"] == scenario.as_str()
                            })
                            .filter_map(|result| result["report"][metric].as_f64())
                            .collect(),
                    )
                })
                .collect();
            text.push_str(&format!("  {metric:<27}"));
            for ((label, _), value) in options.builds.iter().zip(&medians) {
                match value {
                    Some(value) => text.push_str(&format!("  {label}={value:>9.3}")),
                    None => text.push_str(&format!("  {label}=        -")),
                }
            }
            if let (Some(Some(first)), Some(Some(last))) = (medians.first(), medians.last())
                && medians.len() > 1
                && *last != 0.
            {
                text.push_str(&format!("  ×{:.2}", first / last));
            }
            text.push('\n');
        }
        page_opens(&mut text, options, results, scenario);
    }
    text
}

/// `pages-tour`'s opening of each page: the median of its runs' medians, by build.
fn page_opens(text: &mut String, options: &Options, results: &[Value], scenario: &str) {
    let mut pages: Vec<String> = Vec::new();
    for result in results
        .iter()
        .filter(|r| r["report"]["scenario"] == scenario)
    {
        for page in result["report"]["page_open_ms"]
            .as_object()
            .into_iter()
            .flat_map(|pages| pages.keys())
        {
            if !pages.contains(page) {
                pages.push(page.clone());
            }
        }
    }
    for page in pages {
        text.push_str(&format!("  open {page:<22}"));
        let medians: Vec<Option<f64>> = options
            .builds
            .iter()
            .map(|(label, _)| {
                median(
                    results
                        .iter()
                        .filter(|r| {
                            r["build"] == label.as_str() && r["report"]["scenario"] == scenario
                        })
                        .filter_map(|r| r["report"]["page_open_ms"][page.as_str()].as_f64())
                        .collect(),
                )
            })
            .collect();
        for ((label, _), value) in options.builds.iter().zip(&medians) {
            match value {
                Some(value) => text.push_str(&format!("  {label}={value:>9.3}")),
                None => text.push_str(&format!("  {label}=        -")),
            }
        }
        if let (Some(Some(first)), Some(Some(last))) = (medians.first(), medians.last())
            && medians.len() > 1
            && *last != 0.
        {
            text.push_str(&format!("  ×{:.2}", first / last));
        }
        text.push('\n');
    }
}
