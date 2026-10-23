//! CLI probe for native M1 tests.
//!
//! Safety model (code-enforced, not conventional):
//!
//! - `frame <pid>` is read-only and stays open for any pid. It never reads
//!   or prints window titles.
//! - `set-frame` / `raise` are MUTATING and require `--target <pid>
//!   --token <hex>`. The token must match the registration file the Swift
//!   spike harness writes at `~/.hypr-rust/fixture-target.json` (owner-only,
//!   regular, non-symlink, perm 0600). Before any AX mutation the probe
//!   verifies: file exists, is a regular non-symlink file owned by the
//!   current user with perm 0600, pid and token match, the pid is alive, and
//!   the fixture liveness lock (`~/.hypr-rust/fixture.lock`) is held by a
//!   live process (so a stale record or reused PID cannot authorize
//!   mutation). Anything else fails with exit 5 (`unregistered target`) — a
//!   probe cannot mutate an arbitrary pid.
//!
//! `batch` applies a multi-window plan with **best-effort** rollback
//! ([ADR-0005](../adr/0005-batch-multiwindow-best-effort-rollback.md)): the
//! registered fixture must appear in the plan, every target must be alive, and
//! the plan file must be owner-only. On the first failure the batch stops and
//! restores every already-changed window, reporting the actual state honestly;
//! it never claims atomicity.
//!
//! Usage:
//!   ax-probe frame <pid>
//!   ax-probe set-frame --target <pid> --token <hex> <x> <y> <width> <height>
//!   ax-probe raise --target <pid> --token <hex>
//!   ax-probe batch --target <pid> --token <hex> --plan <path>
//!
//! Exit codes: 0 ok · 1 error · 3 permission denied · 4 stale target ·
//! 5 unregistered target.

use std::path::PathBuf;
use std::process::ExitCode;

use hypr_rust_platform_macos::{
    batch::{self, BatchResult, PlanError, WindowOutcome},
    fixture, process_alive, AxError, FixtureError, MacAxAdapter,
};
use layout_core::Rect;

/// Probe-level errors: adapter failures plus the registration failure.
enum ProbeError {
    Ax(AxError),
    /// Token/identity verification failed: target is not the registered
    /// self-owned fixture. Mutations are refused before any AX call.
    TargetUnregistered,
    /// The batch plan file is missing, unreadable, not owner-only, or invalid.
    Plan(PlanError),
    /// A batch ran and failed; the full result is reported.
    Batch(BatchResult),
}

impl From<AxError> for ProbeError {
    fn from(err: AxError) -> Self {
        Self::Ax(err)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(ProbeError::TargetUnregistered) => {
            eprintln!("error: unregistered target (fixture identity verification failed)");
            eprintln!("       mutations require --target <pid> --token <hex> from the harness");
            ExitCode::from(5)
        }
        Err(ProbeError::Plan(err)) => {
            eprintln!("error: invalid batch plan ({err:?})");
            ExitCode::FAILURE
        }
        Err(ProbeError::Batch(result)) => {
            print_batch_result(&result);
            match result.error {
                Some(AxError::PermissionDenied) => ExitCode::from(3),
                Some(AxError::StaleTarget) => ExitCode::from(4),
                _ => ExitCode::FAILURE,
            }
        }
        Err(ProbeError::Ax(AxError::PermissionDenied)) => {
            eprintln!("error: permission denied (Accessibility not granted)");
            ExitCode::from(3)
        }
        Err(ProbeError::Ax(AxError::StaleTarget)) => {
            eprintln!("error: stale target (process/window gone)");
            ExitCode::from(4)
        }
        Err(ProbeError::Ax(AxError::InvalidGeometry)) => {
            eprintln!("error: invalid geometry (non-finite or non-positive size)");
            ExitCode::FAILURE
        }
        Err(ProbeError::Ax(AxError::VerificationFailed { actual, .. })) => {
            eprintln!("error: read-back mismatch; window frame: {actual:?}");
            eprintln!("       see adapter error for expected frame and rollback status");
            ExitCode::FAILURE
        }
        Err(ProbeError::Ax(other)) => {
            eprintln!("error: {other}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), ProbeError> {
    if args.len() < 3 {
        usage();
        return Err(ProbeError::Ax(AxError::IllegalArgument));
    }
    let command = args[1].as_str();

    match command {
        "frame" => {
            // Read-only: no token required.
            let pid: u32 = args[2].parse().map_err(|_| AxError::IllegalArgument)?;
            let adapter = MacAxAdapter::connect()?;
            let window = adapter.main_window(pid)?;
            let rect = adapter.window_frame(&window)?;
            print_frame(&rect);
            Ok(())
        }
        "set-frame" | "raise" => {
            let (pid, token) = parse_target_args(args)?;
            fixture::verify_registration(pid, &token).map_err(map_fixture_error)?;
            let adapter = MacAxAdapter::connect()?;
            let window = adapter.main_window(pid)?;
            match command {
                "set-frame" => {
                    // Geometry = the four positional args after the flags.
                    let mut nums: Vec<f64> = Vec::new();
                    let mut i = 2;
                    while i < args.len() {
                        match args[i].as_str() {
                            "--target" | "--token" => i += 2,
                            _ => {
                                nums.push(args[i].parse().map_err(|_| AxError::IllegalArgument)?);
                                i += 1;
                            }
                        }
                    }
                    if nums.len() != 4 {
                        usage();
                        return Err(ProbeError::Ax(AxError::IllegalArgument));
                    }
                    let rect = Rect::new(nums[0], nums[1], nums[2], nums[3]);
                    // The adapter verifies by read-back internally and returns
                    // the verified frame; no extra post-transaction read here.
                    let verified = adapter.set_window_frame(&window, rect)?;
                    print_frame(&verified);
                    Ok(())
                }
                _ => {
                    adapter.raise(&window)?;
                    println!("ok raised");
                    Ok(())
                }
            }
        }
        "batch" => {
            // Mutating: token-gated like the other mutations. The registered
            // fixture must be part of the plan so the token authorizes the
            // batch; every other target must be a live self-owned fixture.
            let (pid, token) = parse_target_args(args)?;
            fixture::verify_registration(pid, &token).map_err(map_fixture_error)?;
            let plan_path = parse_plan_arg(args)?;
            batch::verify_plan_file(&plan_path).map_err(ProbeError::Plan)?;
            let text = std::fs::read_to_string(&plan_path)
                .map_err(|_| ProbeError::Plan(PlanError::Parse))?;
            let targets = batch::parse_plan(&text).map_err(ProbeError::Plan)?;
            if !targets
                .iter()
                .any(|target| target.window == batch::WindowKey(pid))
            {
                return Err(ProbeError::Plan(PlanError::FixtureNotIncluded));
            }
            for target in &targets {
                if !process_alive(target.window.0) {
                    return Err(ProbeError::Ax(AxError::StaleTarget));
                }
            }
            let adapter = MacAxAdapter::connect()?;
            let result = batch::apply_batch(&adapter, &targets);
            if result.error.is_some() {
                return Err(ProbeError::Batch(result));
            }
            print_batch_result(&result);
            Ok(())
        }
        other => {
            eprintln!("unknown command: {other}");
            usage();
            Err(ProbeError::Ax(AxError::IllegalArgument))
        }
    }
}

/// Parse `--target <pid> --token <hex>` for mutation commands. Missing or
/// malformed target identity is an unregistered-target failure (exit 5):
/// the command must fail before any AX mutation can even be considered.
fn parse_target_args(args: &[String]) -> Result<(u32, String), ProbeError> {
    let mut pid: Option<u32> = None;
    let mut token: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--target" => {
                i += 1;
                pid = Some(
                    args.get(i)
                        .ok_or(ProbeError::TargetUnregistered)?
                        .parse()
                        .map_err(|_| ProbeError::TargetUnregistered)?,
                );
            }
            "--token" => {
                i += 1;
                token = Some(args.get(i).ok_or(ProbeError::TargetUnregistered)?.clone());
            }
            _ => {}
        }
        i += 1;
    }
    match (pid, token) {
        (Some(pid), Some(token)) => Ok((pid, token)),
        _ => Err(ProbeError::TargetUnregistered),
    }
}

/// Parse `--plan <path>` for the batch command.
fn parse_plan_arg(args: &[String]) -> Result<PathBuf, ProbeError> {
    let mut i = 2;
    while i < args.len() {
        if args[i] == "--plan" {
            if let Some(path) = args.get(i + 1) {
                return Ok(PathBuf::from(path));
            }
        }
        i += 1;
    }
    Err(ProbeError::Plan(PlanError::Parse))
}

/// Print a batch result: one line per target plus the rollback status.
fn print_batch_result(result: &BatchResult) {
    let applied = result
        .outcomes
        .iter()
        .filter(|(_, outcome)| matches!(outcome, WindowOutcome::Applied { .. }))
        .count();
    let failed_at = result
        .failed_at
        .map(|key| key.0.to_string())
        .unwrap_or_else(|| "none".to_string());
    let status = if result.error.is_some() {
        "failed"
    } else {
        "ok"
    };
    println!(
        "batch: {status} applied={applied} failed_at={failed_at} rollback_all_restored={}",
        result.rollback_all_restored
    );
    for (window, outcome) in &result.outcomes {
        match outcome {
            WindowOutcome::Applied { requested, actual } => {
                println!(
                    "  pid={} applied requested={requested:?} actual={actual:?}",
                    window.0
                );
            }
            WindowOutcome::RolledBack { restored, actual } => {
                println!(
                    "  pid={} rolled_back restored={restored:?} actual={actual:?}",
                    window.0
                );
            }
        }
    }
    if let Some(error) = &result.error {
        println!("  error: {error}");
    }
}

/// Map a fixture verification failure to a probe error.
fn map_fixture_error(err: FixtureError) -> ProbeError {
    match err {
        FixtureError::Unregistered => ProbeError::TargetUnregistered,
        FixtureError::Stale => ProbeError::Ax(AxError::StaleTarget),
    }
}

fn print_frame(rect: &Rect) {
    println!(
        "ok {{x: {}, y: {}, width: {}, height: {}}}",
        fmt(rect.x),
        fmt(rect.y),
        fmt(rect.width),
        fmt(rect.height)
    );
}

fn usage() {
    eprintln!(
        "usage: ax-probe frame <pid>\n       ax-probe set-frame --target <pid> --token <hex> <x> <y> <width> <height>\n       ax-probe raise --target <pid> --token <hex>\n       ax-probe batch --target <pid> --token <hex> --plan <path>   # plan lines: \"pid x y width height\""
    );
}

/// Print f64 without trailing noise; integers render as integers.
fn fmt(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}
