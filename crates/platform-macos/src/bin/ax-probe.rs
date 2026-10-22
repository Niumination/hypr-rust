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
//! Usage:
//!   ax-probe frame <pid>
//!   ax-probe set-frame --target <pid> --token <hex> <x> <y> <width> <height>
//!   ax-probe raise --target <pid> --token <hex>
//!
//! Exit codes: 0 ok · 1 error · 3 permission denied · 4 stale target ·
//! 5 unregistered target.

use std::process::ExitCode;

use hypr_rust_platform_macos::{fixture, AxError, FixtureError, MacAxAdapter};
use layout_core::Rect;

/// Probe-level errors: adapter failures plus the registration failure.
enum ProbeError {
    Ax(AxError),
    /// Token/identity verification failed: target is not the registered
    /// self-owned fixture. Mutations are refused before any AX call.
    TargetUnregistered,
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
        "usage: ax-probe frame <pid>\n       ax-probe set-frame --target <pid> --token <hex> <x> <y> <width> <height>\n       ax-probe raise --target <pid> --token <hex>"
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
