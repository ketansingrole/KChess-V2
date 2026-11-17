//! `bench`: the GPUI app's frame benchmarks (`apps/gpui/src/bench.rs`) for one or more builds.
//! Each run gets a fresh throwaway profile; the builds alternate run by run (odd runs reverse the
//! order), so a change in the machine's load falls on all of them; the report gives medians.
//!
//! A build is the binary of `cargo build --release -p kchess-gpui --features bench`
//! (`target/release/KChess`), copied aside, since the next build replaces it. Nothing else should
//! run while measuring. A run whose window was covered for a while (`visible_fraction` below 1)
//! draws no frames then; it is reported and left out.
//!
//! `--fail-above <ratio>` makes it a gate: it fails when the first build's frames cost over
//! `ratio` times the last build's in any scenario (`GATED`), or a scenario drew no frames
//! (docs/benchmarks/frame-guards.md).

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
    fail_above: Option<f64>,
}

/// The figures the gate compares, each with the least rise that counts: a frame of 0.3 ms
/// that doubles is noise on a shared runner, one of 1.4 ms that grows to 9 is not (#211, the
/// voice bar asking macOS about the microphone on every frame).
const GATED: [(&str, f64); 2] = [("draw_ms_p50", 1.), ("main_thread_busy_fraction", 0.1)];

fn options(args: &[String]) -> Result<Options> {
    let mut options = Options {
        builds: Vec::new(),
        runs: 5,
        seconds: 30,
        scenarios: SCENARIOS.iter().map(|name| name.to_string()).collect(),
        out: None,
        fail_above: None,
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
            "--fail-above" => {
                let ratio: f64 = value()?.parse().map_err(|_| "--fail-above is a ratio.")?;
                if ratio <= 1. {
                    return Err("--fail-above is a ratio over 1.".into());
                }
                options.fail_above = Some(ratio);
            }
            other => return Err(format!("Unknown bench option {other}.")),
        }
    }
    if options.builds.is_empty() {
        return Err("Name at least one --build label=binary.".into());
    }
    Ok(options)
}

/// `cargo xtask bench --build <label>=<binary> [--build …] [--runs 5] [--seconds 30]
/// [--scenarios a,b] [--out results.json] [--fail-above <ratio>]`
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
    gate(&options, &results)
}

/// With `--fail-above`, an error naming each regression (`regressions`).
fn gate(options: &Options, results: &[Value]) -> Result {
    let Some(ratio) = options.fail_above else {
        return Ok(());
    };
    let found = regressions(options, results, ratio);
    if found.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Frames cost more on {} than on {}:\n  {}",
        options.builds[0].0,
        options.builds[options.builds.len() - 1].0,
        found.join("\n  ")
    ))
}

/// What the gate fails on: each scenario's `GATED` figure whose median on the first build is over
/// `ratio` times the last build's and over it by more than its least rise, or that either build
/// never measured, and each scenario that drew no frames on a build: one that drew nothing proves
/// nothing, and its times are 0 (a GitHub macOS runner draws none; frame-guards.md).
fn regressions(options: &Options, results: &[Value], ratio: f64) -> Vec<String> {
    let (new, base) = (
        &options.builds[0].0,
        &options.builds[options.builds.len() - 1].0,
    );
    let figure = |label: &str, scenario: &str, metric: &str| {
        median(
            results
                .iter()
                .filter(|r| r["build"] == label && r["report"]["scenario"] == scenario)
                .filter_map(|r| r["report"][metric].as_f64())
                .collect(),
        )
    };
    let mut found = Vec::new();
    for scenario in &options.scenarios {
        for label in [new, base] {
            if figure(label, scenario, "draws_per_s").is_some_and(|draws| draws <= 0.) {
                found.push(format!("{scenario}: drew no frames on {label}"));
            }
        }
        for (metric, least) in GATED {
            match (
                figure(new, scenario, metric),
                figure(base, scenario, metric),
            ) {
                (Some(now), Some(was)) if now > was * ratio && now - was > least => {
                    found.push(format!(
                        "{scenario} {metric}: {now:.3} against {was:.3} (×{:.2})",
                        now / was
                    ))
                }
                (Some(_), Some(_)) => {}
                _ => found.push(format!("{scenario} {metric}: not measured on both builds")),
            }
        }
    }
    found
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
            .env("KCHESS_ENGINE_PATH", engine)
            .env("KCHESS_BENCH", bench);
        command
    };
    if lichess.is_some() {
        // Signed in by an earlier launch, as someone who plays on Lichess is.
        app("lichess-login")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("Could not start {}: {e}", binary.display()))?;
    }
    let status = app(scenario)
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
            text.push_str(&row(options, &medians));
        }
        page_opens(
            &mut text,
            options,
            results,
            scenario,
            "first_open_ms",
            "first",
        );
        page_opens(
            &mut text,
            options,
            results,
            scenario,
            "page_open_ms",
            "open",
        );
    }
    text
}

/// `pages-tour`'s opening of each page (`key`: the first, or again): the median of its runs',
/// by build.
fn page_opens(
    text: &mut String,
    options: &Options,
    results: &[Value],
    scenario: &str,
    key: &str,
    title: &str,
) {
    let mut pages: Vec<String> = Vec::new();
    for result in results
        .iter()
        .filter(|r| r["report"]["scenario"] == scenario)
    {
        for page in result["report"][key]
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
        text.push_str(&format!("  {title} {page:<22}"));
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
                        .filter_map(|r| r["report"][key][page.as_str()].as_f64())
                        .collect(),
                )
            })
            .collect();
        text.push_str(&row(options, &medians));
    }
}

/// One figure's medians, a column per build, then the first build's over the last's, and the
/// line's end.
fn row(options: &Options, medians: &[Option<f64>]) -> String {
    let mut text = String::new();
    for ((label, _), value) in options.builds.iter().zip(medians) {
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
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_median_is_the_middle_run_or_the_mean_of_the_two() {
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![3., 1., 2.]), Some(2.));
        assert_eq!(median(vec![4., 1., 3., 2.]), Some(2.5));
    }

    /// Each scenario's figures are the medians of its runs, by build, with the first build's
    /// over the last's; a figure a build never reported shows as a dash.
    #[test]
    fn the_summary_compares_the_builds_medians() {
        let options = Options {
            builds: vec![("new".into(), "a".into()), ("old".into(), "b".into())],
            runs: 3,
            seconds: 1,
            scenarios: vec!["home-idle".into()],
            out: None,
            fail_above: None,
        };
        let run = |build: &str, draws: f64| {
            json!({
                "build": build,
                "report": {"scenario": "home-idle", "draws_per_s": draws, "page_open_ms": {"play": draws}}
            })
        };
        let results = [
            run("new", 60.),
            run("new", 30.),
            run("new", 50.),
            run("old", 25.),
        ];
        let text = summary(&options, &results);
        assert!(text.starts_with("\nhome-idle\n"), "{text}");
        assert!(
            text.contains(&format!(
                "  {:<27}  new=   50.000  old=   25.000  ×2.00\n",
                "draws_per_s"
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "  {:<27}  new=        -  old=        -\n",
                "latency_ms_p50"
            )),
            "{text}"
        );
        assert!(
            text.contains(&format!(
                "  open {:<22}  new=   50.000  old=   25.000  ×2.00\n",
                "play"
            )),
            "{text}"
        );
    }

    fn gated(build: &str, draw: f64, busy: f64) -> Value {
        json!({
            "build": build,
            "report": {"scenario": "analysis-stepping", "draw_ms_p50": draw, "main_thread_busy_fraction": busy}
        })
    }

    fn two_builds() -> Options {
        Options {
            builds: vec![("main".into(), "a".into()), ("base".into(), "b".into())],
            runs: 1,
            seconds: 1,
            scenarios: vec!["analysis-stepping".into()],
            out: None,
            fail_above: Some(2.),
        }
    }

    /// #211 on CI's terms: the analysis board's frames went from 1.4 to 9 ms and its main thread
    /// from 17% to 91% busy.
    #[test]
    fn the_gate_fails_on_the_voice_bar_regression() {
        let results = [gated("main", 9.15, 0.91), gated("base", 1.38, 0.17)];
        assert_eq!(
            regressions(&two_builds(), &results, 2.),
            [
                "analysis-stepping draw_ms_p50: 9.150 against 1.380 (×6.63)",
                "analysis-stepping main_thread_busy_fraction: 0.910 against 0.170 (×5.35)",
            ]
        );
    }

    /// Faster, a little slower, or twice as slow by less than the least rise: no regression.
    #[test]
    fn the_gate_passes_noise_and_gains() {
        for (draw, busy) in [(0.9, 0.1), (1.38, 0.17), (2.7, 0.33), (0.6, 0.08)] {
            let results = [gated("main", draw, busy), gated("base", 1.38, 0.17)];
            assert!(
                regressions(&two_builds(), &results, 2.).is_empty(),
                "{draw} {busy}"
            );
        }
        // Small frames that doubled: under the least rise.
        let results = [gated("main", 0.9, 0.05), gated("base", 0.3, 0.02)];
        assert!(regressions(&two_builds(), &results, 2.).is_empty());
        // Over the ratio by more than the least rise in only one figure.
        let results = [gated("main", 2.5, 0.2), gated("base", 1., 0.15)];
        assert_eq!(
            regressions(&two_builds(), &results, 2.),
            ["analysis-stepping draw_ms_p50: 2.500 against 1.000 (×2.50)"]
        );
        // Exactly at the ratio is not over it.
        let results = [gated("main", 4., 0.2), gated("base", 2., 0.15)];
        assert!(regressions(&two_builds(), &results, 2.).is_empty());
        // Exactly the least rise over is not more than it.
        let results = [gated("main", 1.5, 0.15), gated("base", 0.5, 0.05)];
        assert!(regressions(&two_builds(), &results, 2.).is_empty());
    }

    #[test]
    fn the_gate_fails_only_when_asked() {
        let slower = [gated("main", 9.15, 0.91), gated("base", 1.38, 0.17)];
        let same = [gated("main", 1.38, 0.17), gated("base", 1.38, 0.17)];
        let error = gate(&two_builds(), &slower).unwrap_err();
        assert!(
            error.starts_with(
                "Frames cost more on main than on base:\n  analysis-stepping draw_ms_p50"
            ),
            "{error}"
        );
        assert!(gate(&two_builds(), &same).is_ok());
        let unasked = Options {
            fail_above: None,
            ..two_builds()
        };
        assert!(gate(&unasked, &slower).is_ok());
    }

    /// What a GitHub macOS runner reports: no frames, so times of 0 on both builds.
    #[test]
    fn the_gate_fails_a_scenario_that_drew_no_frames() {
        let idle = |build: &str, draws: f64| {
            json!({
                "build": build,
                "report": {"scenario": "analysis-stepping", "draws_per_s": draws,
                    "draw_ms_p50": 0., "main_thread_busy_fraction": 0.002}
            })
        };
        assert_eq!(
            regressions(&two_builds(), &[idle("main", 0.), idle("base", 0.)], 2.),
            [
                "analysis-stepping: drew no frames on main",
                "analysis-stepping: drew no frames on base",
            ]
        );
        assert_eq!(
            regressions(&two_builds(), &[idle("main", 0.), idle("base", 120.)], 2.),
            ["analysis-stepping: drew no frames on main"]
        );
        assert!(
            regressions(&two_builds(), &[idle("main", 120.), idle("base", 0.5)], 2.).is_empty()
        );
    }

    #[test]
    fn the_gate_fails_a_scenario_that_measured_nothing() {
        let results = [gated("base", 1.38, 0.17)];
        assert_eq!(
            regressions(&two_builds(), &results, 2.),
            [
                "analysis-stepping draw_ms_p50: not measured on both builds",
                "analysis-stepping main_thread_busy_fraction: not measured on both builds",
            ]
        );
    }

    #[test]
    fn the_gate_takes_a_ratio_over_one() {
        let args = |ratio: &str| ["--build", "a=b", "--fail-above", ratio].map(str::to_string);
        assert_eq!(options(&args("2.5")).unwrap().fail_above, Some(2.5));
        assert!(options(&args("1")).is_err());
        assert!(options(&args("fast")).is_err());
        assert_eq!(
            options(&["--build".into(), "a=b".into()])
                .unwrap()
                .fail_above,
            None
        );
    }
}
