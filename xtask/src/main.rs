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
  bench-startup [--warm [--gap-ms MS]] [--runs N] [--budget-ms MS] [--bin PATH] [--no-build]
      Launches the release binary N times (default 20) with --startup-report and
      reports spawn-to-first-frame latency. The first run is reported on its
      own (coldest caches); percentiles cover the other runs. Exits non-zero
      if their p95 exceeds the budget (default 50 ms).
      --warm measures launches handed to a resident instance instead: spawn of
      the second process until the resident instance has drawn the new window.
      --gap-ms waits between those launches (default 500, like a person; the
      resident instance prepares its next window in between).
  dist
      Builds the release binary and packs it with the README, changelog and
      licenses into target/dist/tachyon-<version>-<arch>-<os>.zip on Windows,
      .tar.gz elsewhere.
  icons
      Regenerates the application's icons from assets/brand: tachyon.ico (16-64 px,
      32-bit DIB images; the small icon art up to 32 px) and the launcher SVG, both in
      crates/tachyon-platform/assets. Needs rsvg-convert and ImageMagick (magick).
      Run after changing the brand art; the build only embeds the results.
";

const REPORT_PREFIX: &str = "tachyon-startup";
const RUN_TIMEOUT: Duration = Duration::from_secs(15);

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("ci") => ci(args.collect()),
        Some("bench-startup") => bench_startup(args.collect()),
        Some("dist") => dist(),
        Some("icons") => icons(),
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
    warm: bool,
    gap: Duration,
}

fn parse_bench_options(args: Vec<String>) -> Result<BenchOptions, String> {
    let mut options = BenchOptions {
        runs: 20,
        budget: Duration::from_millis(50),
        bin: None,
        build: true,
        warm: false,
        gap: Duration::from_millis(500),
    };
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
            "--warm" => options.warm = true,
            "--gap-ms" => {
                let ms: u64 = value()?.parse().map_err(|e| format!("--gap-ms: {e}"))?;
                options.gap = Duration::from_millis(ms);
            }
            _ => return Err(format!("unknown argument {arg}\n\n{USAGE}")),
        }
    }
    Ok(options)
}

/// Files shipped next to the binary in a release archive.
const DIST_FILES: [&str; 4] = ["README.md", "CHANGELOG.md", "LICENSE-MIT", "LICENSE-APACHE"];

fn dist() -> Result<ExitCode, String> {
    run_cargo(&["build", "--release", "--locked", "--package", "tachyon"])?;
    let root = workspace_root();
    let version = workspace_version(&root)?;
    let name = format!("tachyon-{version}-{}-{}", std::env::consts::ARCH, std::env::consts::OS);
    let dist = release_binary()
        .parent()
        .and_then(|release| release.parent())
        .ok_or("no target directory")?
        .join("dist");
    let stage = dist.join(&name);
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
    let binary = release_binary();
    let copy = |from: &std::path::Path| -> Result<(), String> {
        let to = stage.join(from.file_name().ok_or("file without a name")?);
        std::fs::copy(from, &to).map(drop).map_err(|e| format!("{}: {e}", from.display()))
    };
    copy(&binary)?;
    for file in DIST_FILES {
        copy(&root.join(file))?;
    }
    // Windows 10 and later ship bsdtar, which writes zip archives (`-a` picks the format from the
    // extension); elsewhere a gzipped tar is the norm.
    let archive = if cfg!(windows) { format!("{name}.zip") } else { format!("{name}.tar.gz") };
    let mut tar = Command::new("tar");
    if cfg!(windows) {
        tar.args(["-a", "-c", "-f", &archive, &name]);
    } else {
        tar.args(["-c", "-z", "-f", &archive, &name]);
    }
    let status = tar.current_dir(&dist).status().map_err(|e| format!("failed to run tar: {e}"))?;
    if !status.success() {
        return Err("tar failed".into());
    }
    let path = dist.join(&archive);
    let size = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    println!("{} ({:.1} MB)", path.display(), size as f64 / 1e6);
    Ok(ExitCode::SUCCESS)
}

/// Icon sizes and the art drawn at each: the simplified mark stays legible up to 32 px.
const ICON_SIZES: [(u32, &str); 7] = [
    (16, "app-icon-small.svg"),
    (20, "app-icon-small.svg"),
    (24, "app-icon-small.svg"),
    (32, "app-icon-small.svg"),
    (40, "app-icon.svg"),
    (48, "app-icon.svg"),
    (64, "app-icon.svg"),
];

/// `cargo xtask icons`: rasterizes the brand SVGs into the `.ico` the tray, the windows and the
/// executable embed, and copies the launcher SVG. Sizes stay at 64 px and below, which ImageMagick
/// writes as DIB images: the tray creates icons without an image codec and rejects PNG entries.
fn icons() -> Result<ExitCode, String> {
    let root = workspace_root();
    let brand = root.join("assets/brand");
    let out = root.join("crates/tachyon-platform/assets");
    let work = std::env::temp_dir().join(format!("tachyon-icons-{}", std::process::id()));
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let run = |program: &str, args: &[&std::ffi::OsStr]| -> Result<(), String> {
        let status = Command::new(program)
            .args(args)
            .status()
            .map_err(|e| format!("{program} is needed for `cargo xtask icons` ({e})"))?;
        if status.success() { Ok(()) } else { Err(format!("{program} failed")) }
    };
    let mut pngs = Vec::new();
    for (size, art) in ICON_SIZES {
        let png = work.join(format!("{size}.png"));
        let size = size.to_string();
        run(
            "rsvg-convert",
            &[
                "-w".as_ref(),
                size.as_ref(),
                "-h".as_ref(),
                size.as_ref(),
                brand.join(art).as_os_str(),
                "-o".as_ref(),
                png.as_os_str(),
            ],
        )?;
        pngs.push(png);
    }
    let ico = out.join("tachyon.ico");
    let mut args: Vec<&std::ffi::OsStr> = pngs.iter().map(|p| p.as_os_str()).collect();
    args.push(ico.as_os_str());
    run("magick", &args)?;
    let _ = std::fs::remove_dir_all(&work);
    check_dib_icon(&std::fs::read(&ico).map_err(|e| e.to_string())?)?;
    let svg = out.join("tachyon.svg");
    std::fs::copy(brand.join("app-icon.svg"), &svg)
        .map_err(|e| format!("{}: {e}", svg.display()))?;
    println!("{} and {} updated", ico.display(), svg.display());
    Ok(ExitCode::SUCCESS)
}

/// Every image in the `.ico` is a DIB (a `BITMAPINFOHEADER`, not a PNG).
fn check_dib_icon(ico: &[u8]) -> Result<(), String> {
    let count = usize::from(u16::from_le_bytes([ico[4], ico[5]]));
    for i in 0..count {
        let entry = &ico[6 + 16 * i..22 + 16 * i];
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
        if ico.get(offset..offset + 4) != Some(&[40, 0, 0, 0][..]) {
            return Err(format!("icon image {i} is not a 32-bit DIB"));
        }
    }
    Ok(())
}

/// `version` from the workspace manifest's `[workspace.package]` table.
fn workspace_version(root: &std::path::Path) -> Result<String, String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    manifest
        .split("[workspace.package]")
        .nth(1)
        .and_then(|table| table.lines().find_map(|line| line.trim().strip_prefix("version = ")))
        .map(|version| version.trim_matches('"').to_owned())
        .ok_or_else(|| "no version in [workspace.package]".into())
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

    if options.warm {
        let samples = bench_warm(&bin, options.runs, options.gap)?;
        return Ok(report(&samples, options.budget, "receipt"));
    }
    let mut samples = Vec::with_capacity(options.runs);
    for run in 1..=options.runs {
        let sample = launch_once(&bin).map_err(|e| format!("run {run}: {e}"))?;
        samples.push(sample);
    }
    Ok(report(&samples, options.budget, "main"))
}

/// Starts a resident instance (on a private instance id, so a running
/// Tachyon is not involved), then times launches handed to it.
fn bench_warm(bin: &PathBuf, runs: usize, gap: Duration) -> Result<Vec<Sample>, String> {
    let id = format!("tachyon-bench-{}", std::process::id());
    let mut resident = Command::new(bin)
        .arg("--report-launches")
        .env("TACHYON_INSTANCE_ID", &id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to launch {}: {e}", bin.display()))?;
    let stdout = resident.stdout.take().expect("stdout is piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send((Instant::now(), line)).is_err() {
                break;
            }
        }
    });

    let result = (|| {
        wait_for_line(&rx, "tachyon-ready")?;
        let mut samples = Vec::with_capacity(runs);
        for run in 1..=runs {
            std::thread::sleep(gap);
            let started = Instant::now();
            let status = Command::new(bin)
                .env("TACHYON_INSTANCE_ID", &id)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .status()
                .map_err(|e| format!("run {run}: failed to launch: {e}"))?;
            if !status.success() {
                return Err(format!("run {run}: launch exited with {status}"));
            }
            let (at, line) =
                wait_for_line(&rx, "tachyon-launch").map_err(|e| format!("run {run}: {e}"))?;
            samples.push(Sample { external: at - started, marks: parse_report(&line)? });
        }
        Ok(samples)
    })();
    let _ = resident.kill();
    let _ = resident.wait();
    result
}

fn wait_for_line(
    rx: &mpsc::Receiver<(Instant, String)>,
    prefix: &str,
) -> Result<(Instant, String), String> {
    let deadline = Instant::now() + RUN_TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((at, line)) if line.starts_with(prefix) => return Ok((at, line)),
            Ok(_) => continue,
            Err(_) => return Err(format!("resident instance did not print `{prefix}`")),
        }
    }
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

/// `marks_from` names where the in-process milestones are measured from.
fn report(samples: &[Sample], budget: Duration, marks_from: &str) -> ExitCode {
    let ms = |d: Duration| format!("{:>8.2}", d.as_secs_f64() * 1000.0);
    let mut rows: Vec<(String, Vec<Duration>)> =
        vec![("spawn -> first frame".into(), samples.iter().map(|s| s.external).collect())];
    for name in samples[0].marks.keys() {
        let values = samples.iter().filter_map(|s| s.marks.get(name).copied()).collect();
        rows.push((format!("{marks_from} -> {name}"), values));
    }

    // The first launch has the coldest caches: it gets its own column, and
    // the percentiles (and the verdict) cover the remaining runs.
    let warm = samples.len() > 1;
    let rest_of = |values: &[Duration]| {
        let mut rest = if warm { values[1..].to_vec() } else { values.to_vec() };
        rest.sort();
        rest
    };
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
        let rest = rest_of(values);
        println!(
            "{name:<24} {} {} {} {}",
            ms(first),
            ms(percentile(&rest, 0.50)),
            ms(percentile(&rest, 0.95)),
            ms(*rest.last().expect("non-empty"))
        );
    }

    // Same statistic as the table's p95 column for the first row.
    let p95 = percentile(&rest_of(&rows[0].1), 0.95);
    let verdict = if p95 <= budget { "PASS" } else { "FAIL" };
    let runs = if warm { format!("runs 2-{}", samples.len()) } else { "1 run".to_owned() };
    println!(
        "\n{verdict}: p95 spawn -> first frame {:.2} ms (budget {} ms, {runs})",
        p95.as_secs_f64() * 1000.0,
        budget.as_millis()
    );
    if p95 <= budget { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
