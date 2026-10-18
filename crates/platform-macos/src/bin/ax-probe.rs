//! CLI probe for native M1 tests.
//!
//! Safety model (code-enforced, not conventional):
//!
//! - `frame <pid>` is read-only and stays open for any pid. It never reads
//!   or prints window titles.
//! - `set-frame` / `raise` are MUTATING and require `--target <pid>
//!   --token <hex>`. The token must match the registration file the Swift
//!   spike harness writes at `/tmp/hypr-rust-target.json` (perm 0600) when
//!   it spawns the self-owned `HYPR-SPIKE-TARGET` fixture. Before any AX
//!   mutation the probe verifies: file exists, permissions are 0600, pid and
//!   token match, and the pid is alive. Anything else fails with exit 5
//!   (`unregistered target`) — a probe cannot mutate an arbitrary pid.
//!
//! Usage:
//!   ax-probe frame <pid>
//!   ax-probe set-frame --target <pid> --token <hex> <x> <y> <width> <height>
//!   ax-probe raise --target <pid> --token <hex>
//!
//! Exit codes: 0 ok · 1 error · 3 permission denied · 4 stale target ·
//! 5 unregistered target.

use std::process::ExitCode;

use hypr_rust_platform_macos::{process_alive, AxError, MacAxAdapter};
use layout_core::Rect;

/// Target registration file written by the Swift spike harness
/// (spikes/macos/spike-macos.swift, `spawn-target`). Must stay 0600.
const TARGET_FILE: &str = "/tmp/hypr-rust-target.json";

/// Probe-level errors: adapter failures plus the registration failure.
enum ProbeError {
    Ax(AxError),
    /// Token verification failed: target is not the registered self-owned
    /// fixture. Mutations are refused before any AX call.
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
            eprintln!("error: unregistered target (token verification failed)");
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
            verify_target_registration(pid, &token)?;
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
                    adapter.set_window_frame(&window, rect)?;
                    // Defense-in-depth read-back (the adapter already verifies
                    // internally; this catches a race between verify and now).
                    let after = adapter.window_frame(&window)?;
                    if (after.x - rect.x).abs() > 1.0
                        || (after.y - rect.y).abs() > 1.0
                        || (after.width - rect.width).abs() > 1.0
                        || (after.height - rect.height).abs() > 1.0
                    {
                        eprintln!("error: read-back mismatch after set-frame: {after:?}");
                        return Err(ProbeError::Ax(AxError::Failure(-25204)));
                    }
                    print_frame(&after);
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

/// Parse `--target <pid> --token <hex>` for mutation commands.
fn parse_target_args(args: &[String]) -> Result<(u32, String), ProbeError> {
    // Missing or malformed target identity is an unregistered-target
    // failure (exit 5): the command must fail before any AX mutation can
    // even be considered.
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

/// Verify the target registration file written by the Swift harness:
/// exists, perm 0600, contains matching pid + token, pid alive.
fn verify_target_registration(pid: u32, token: &str) -> Result<(), ProbeError> {
    let path = std::path::Path::new(TARGET_FILE);
    let meta = std::fs::metadata(path).map_err(|_| ProbeError::TargetUnregistered)?;
    // Permission check: must be exactly 0600 (owner-only).
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(ProbeError::TargetUnregistered);
    }
    let body = std::fs::read_to_string(path).map_err(|_| ProbeError::TargetUnregistered)?;
    let (file_pid, file_token) = parse_registration(&body).ok_or(ProbeError::TargetUnregistered)?;
    if file_pid != pid || file_token != token {
        return Err(ProbeError::TargetUnregistered);
    }
    if !process_alive(pid) {
        return Err(ProbeError::Ax(AxError::StaleTarget));
    }
    Ok(())
}

/// Minimal parser for the harness-written registration JSON
/// (`{"pid":<u32>,"token":"<hex>"}`). No dependencies.
fn parse_registration(body: &str) -> Option<(u32, String)> {
    let pid_start = body.find("\"pid\":")? + "\"pid\":".len();
    let pid_end = body[pid_start..].find(|c: char| !c.is_ascii_digit())? + pid_start;
    let pid: u32 = body[pid_start..pid_end].parse().ok()?;
    let token_start = body.find("\"token\":\"")? + "\"token\":\"".len();
    let token_end = body[token_start..].find('"')? + token_start;
    let token = body[token_start..token_end].to_string();
    if token.is_empty() {
        return None;
    }
    Some((pid, token))
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
