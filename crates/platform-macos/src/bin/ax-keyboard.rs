//! M2 CLI: explicit keyboard-first actions on the registered fixture.
//!
//! Safety model: every action requires `--target <pid> --token <hex>` and
//! verifies the fixture registration (token + flock lock) before any AX call.
//! Actions are explicit one-shot operations — no background loop, no global
//! hotkey, no auto-tiling. Keyboard input is received through this focused
//! CLI (or a UI that drives it), not via a system-wide hotkey.
//!
//! Usage:
//!   ax-keyboard select  --target <pid> --token <hex>
//!   ax-keyboard move    --target <pid> --token <hex> <x> <y>
//!   ax-keyboard resize  --target <pid> --token <hex> <width> <height>
//!   ax-keyboard raise   --target <pid> --token <hex>
//!
//! Exit codes: 0 ok · 1 error · 3 permission denied · 4 stale target ·
//! 5 unregistered target.

use std::process::ExitCode;

use hypr_rust_platform_macos::{fixture, AxError, FixtureError, MacAxAdapter};
use layout_core::Rect;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(KeyboardError::Unregistered) => {
            eprintln!("error: unregistered target (fixture identity verification failed)");
            eprintln!("       actions require --target <pid> --token <hex> from the harness");
            ExitCode::from(5)
        }
        Err(KeyboardError::Ax(AxError::PermissionDenied)) => {
            eprintln!("error: permission denied (Accessibility not granted)");
            ExitCode::from(3)
        }
        Err(KeyboardError::Ax(AxError::StaleTarget)) => {
            eprintln!("error: stale target (process/window gone)");
            ExitCode::from(4)
        }
        Err(KeyboardError::Ax(AxError::InvalidGeometry)) => {
            eprintln!("error: invalid geometry (non-finite or non-positive size)");
            ExitCode::FAILURE
        }
        Err(KeyboardError::Ax(AxError::VerificationFailed { actual, .. })) => {
            eprintln!("error: read-back mismatch; window frame: {actual:?}");
            eprintln!("       see adapter error for expected frame and rollback status");
            ExitCode::FAILURE
        }
        Err(KeyboardError::Ax(other)) => {
            eprintln!("error: {other}");
            ExitCode::FAILURE
        }
    }
}

enum KeyboardError {
    Unregistered,
    Ax(AxError),
}

impl From<AxError> for KeyboardError {
    fn from(err: AxError) -> Self {
        Self::Ax(err)
    }
}

impl From<FixtureError> for KeyboardError {
    fn from(err: FixtureError) -> Self {
        match err {
            FixtureError::Unregistered => Self::Unregistered,
            FixtureError::Stale => Self::Ax(AxError::StaleTarget),
        }
    }
}

fn run(args: &[String]) -> Result<(), KeyboardError> {
    if args.len() < 2 {
        usage();
        return Err(KeyboardError::Ax(AxError::IllegalArgument));
    }
    let command = args[1].as_str();
    match command {
        "select" => {
            let (pid, token) = parse_target_args(args)?;
            fixture::verify_registration(pid, &token)?;
            let adapter = MacAxAdapter::connect()?;
            let window = adapter.main_window(pid)?;
            let rect = adapter.window_frame(&window)?;
            println!("ok fixture pid={pid} frame={}", fmt_frame(&rect));
            Ok(())
        }
        "move" | "resize" | "raise" => {
            let (pid, token) = parse_target_args(args)?;
            fixture::verify_registration(pid, &token)?;
            let adapter = MacAxAdapter::connect()?;
            let window = adapter.main_window(pid)?;
            match command {
                "move" => {
                    let (x, y) = parse_two(args)?;
                    // Preserve the current size; move is an explicit position set.
                    let current = adapter.window_frame(&window)?;
                    let rect = Rect::new(x, y, current.width, current.height);
                    let verified = adapter.set_window_frame(&window, rect)?;
                    println!("ok moved {}", fmt_frame(&verified));
                    Ok(())
                }
                "resize" => {
                    let (w, h) = parse_two(args)?;
                    // Preserve the current position; resize is an explicit size set.
                    let current = adapter.window_frame(&window)?;
                    let rect = Rect::new(current.x, current.y, w, h);
                    let verified = adapter.set_window_frame(&window, rect)?;
                    println!("ok resized {}", fmt_frame(&verified));
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
            Err(KeyboardError::Ax(AxError::IllegalArgument))
        }
    }
}

/// Parse `--target <pid> --token <hex>`.
fn parse_target_args(args: &[String]) -> Result<(u32, String), KeyboardError> {
    let mut pid: Option<u32> = None;
    let mut token: Option<String> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--target" => {
                i += 1;
                pid = Some(
                    args.get(i)
                        .ok_or(KeyboardError::Unregistered)?
                        .parse()
                        .map_err(|_| KeyboardError::Unregistered)?,
                );
            }
            "--token" => {
                i += 1;
                token = Some(args.get(i).ok_or(KeyboardError::Unregistered)?.clone());
            }
            _ => {}
        }
        i += 1;
    }
    match (pid, token) {
        (Some(pid), Some(token)) => Ok((pid, token)),
        _ => Err(KeyboardError::Unregistered),
    }
}

/// Parse the two trailing numeric args (x/y or width/height).
fn parse_two(args: &[String]) -> Result<(f64, f64), AxError> {
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
    if nums.len() != 2 {
        return Err(AxError::IllegalArgument);
    }
    Ok((nums[0], nums[1]))
}

fn fmt_frame(rect: &Rect) -> String {
    format!(
        "{{x: {}, y: {}, width: {}, height: {}}}",
        fmt(rect.x),
        fmt(rect.y),
        fmt(rect.width),
        fmt(rect.height)
    )
}

/// Print f64 without trailing noise; integers render as integers.
fn fmt(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn usage() {
    eprintln!(
        "usage: ax-keyboard select --target <pid> --token <hex>\n       ax-keyboard move --target <pid> --token <hex> <x> <y>\n       ax-keyboard resize --target <pid> --token <hex> <width> <height>\n       ax-keyboard raise --target <pid> --token <hex>"
    );
}
