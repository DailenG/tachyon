//! Development tasks. Run with `cargo xtask <task>`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const USAGE: &str = "\
Usage: cargo xtask <TASK>

Tasks:
  ci
      Runs the checks required by CI: rustfmt, clippy, tests and cargo-deny
      (when installed). Run before pushing.
  bench-startup [--runs N] [--budget-ms MS] [--bin PATH] [--no-build]
      Launches the release binary N times (default 20) with --startup-report and
      reports spawn-to-first-frame latency. Exits non-zero if the p95 exceeds
      the budget (default 50 ms).
";

const REPORT_PREFIX: &str = "tachyon-startup";
const RUN_TIMEOUT: Duration = Duration::from_secs(15);

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("ci") => ci(args.collect()),
        Some("bench-startup") => bench_startup(args.collect()),
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cargo() -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(workspace_root());
    command
}

fn run_cargo(args: &[&str]) -> Result<(), String> {
    let line = format!("cargo {}", args.join(" "));
    eprintln!("$ {line}");
    let status = cargo().args(args).status().map_err(|e| format!("failed to run {line}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("`{line}` failed")) }
}

/// Mirrors the required CI jobs so a green local run predicts a green PR.
fn ci(args: Vec<String>) -> Result<ExitCode, String> {
    if let Some(arg) = args.first() {
        return Err(format!("ci takes no arguments, got {arg}"));
    }
    run_cargo(&["fmt", "--all", "--check"])?;
    run_cargo(&["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"])?;
    run_cargo(&["test", "--workspace", "--locked"])?;
    if Command::new("cargo-deny").arg("--version").output().is_ok() {
        run_cargo(&["deny", "check"])?;
    } else {
        eprintln!(
            "xtask: cargo-deny is not installed; `cargo deny check` skipped (CI enforces it).\n\
             Install with: cargo install cargo-deny --locked"
        );
    }
    Ok(ExitCode::SUCCESS)
}

struct BenchOptions {
    runs: usize,
    budget: Duration,
    bin: Option<PathBuf>,
    build: bool,
}

fn parse_bench_options(args: Vec<String>) -> Result<BenchOptions, String> {
    let mut options =
        BenchOptions { runs: 20, budget: Duration::from_millis(50), bin: None, build: true };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--runs" => {
                options.runs = value()?.parse().map_err(|e| format!("--runs: {e}"))?;
                if options.runs == 0 {
                    return Err("--runs must be at least 1".into());
                }
            }
            "--budget-ms" => {
                let ms: u64 = value()?.parse().map_err(|e| format!("--budget-ms: {e}"))?;
                options.budget = Duration::from_millis(ms);
            }
            "--bin" => options.bin = Some(value()?.into()),
            "--no-build" => options.build = false,
            _ => return Err(format!("unknown argument {arg}\n\n{USAGE}")),
        }
    }
    Ok(options)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is inside the workspace")
        .into()
}

fn release_binary() -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"));
    target_dir.join("release").join(format!("tachyon{}", std::env::consts::EXE_SUFFIX))
}

/// One launch: wall time from `spawn` to the report line, plus the in-process
/// milestones the binary printed.
struct Sample {
    external: Duration,
    marks: BTreeMap<String, Duration>,
}

fn bench_startup(args: Vec<String>) -> Result<ExitCode, String> {
    let options = parse_bench_options(args)?;
    if options.build && options.bin.is_none() {
        run_cargo(&["build", "--release", "--package", "tachyon"])?;
    }
    let bin = options.bin.clone().unwrap_or_else(release_binary);

    let mut samples = Vec::with_capacity(options.runs);
    for run in 1..=options.runs {
        let sample = launch_once(&bin).map_err(|e| format!("run {run}: {e}"))?;
        samples.push(sample);
    }
    Ok(report(&samples, options.budget))
}

fn launch_once(bin: &PathBuf) -> Result<Sample, String> {
    let started = Instant::now();
    let mut child = Command::new(bin)
        .arg("--startup-report")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to launch {}: {e}", bin.display()))?;

    let stdout = child.stdout.take().expect("stdout is piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line.starts_with(REPORT_PREFIX) {
                let _ = tx.send((Instant::now(), line));
                break;
            }
        }
    });

    let received = rx.recv_timeout(RUN_TIMEOUT);
    let exited = wait_with_timeout(&mut child, RUN_TIMEOUT);
    let (at, line) = received
        .map_err(|_| "no startup report received (did the window fail to open?)".to_owned())?;
    if !exited {
        return Err("tachyon did not exit after reporting".into());
    }
    Ok(Sample { external: at - started, marks: parse_report(&line)? })
}

fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

fn parse_report(line: &str) -> Result<BTreeMap<String, Duration>, String> {
    line.split_whitespace()
        .skip(1)
        .map(|field| {
            let (key, value) = field.split_once('=').ok_or_else(|| format!("bad field {field}"))?;
            let name = key.strip_suffix("_us").ok_or_else(|| format!("bad field {field}"))?;
            let us: u64 = value.parse().map_err(|e| format!("bad field {field}: {e}"))?;
            Ok((name.to_owned(), Duration::from_micros(us)))
        })
        .collect()
}

/// Nearest-rank percentile of a non-empty, sorted slice.
fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

fn report(samples: &[Sample], budget: Duration) -> ExitCode {
    let ms = |d: Duration| format!("{:>8.2}", d.as_secs_f64() * 1000.0);
    let mut rows: Vec<(String, Vec<Duration>)> =
        vec![("spawn -> first frame".into(), samples.iter().map(|s| s.external).collect())];
    for name in samples[0].marks.keys() {
        let values = samples.iter().filter_map(|s| s.marks.get(name).copied()).collect();
        rows.push((format!("main -> {name}"), values));
    }

    // The first launch has the coldest caches; percentiles cover the rest.
    let warm = samples.len() > 1;
    println!(
        "{:<24}{:>9}{:>9}{:>9}{:>9}   ({} runs, ms)",
        "metric",
        "first",
        "p50",
        "p95",
        "max",
        samples.len()
    );
    for (name, values) in &rows {
        let first = values[0];
        let mut rest = if warm { values[1..].to_vec() } else { values.clone() };
        rest.sort();
        println!(
            "{name:<24} {} {} {} {}",
            ms(first),
            ms(percentile(&rest, 0.50)),
            ms(percentile(&rest, 0.95)),
            ms(*rest.last().expect("non-empty"))
        );
    }

    let mut external: Vec<Duration> = rows[0].1.clone();
    external.sort();
    let p95 = percentile(&external, 0.95);
    let verdict = if p95 <= budget { "PASS" } else { "FAIL" };
    println!(
        "\n{verdict}: p95 spawn -> first frame {:.2} ms (budget {} ms, all runs)",
        p95.as_secs_f64() * 1000.0,
        budget.as_millis()
    );
    if p95 <= budget { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
