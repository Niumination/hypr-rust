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
        Err(KeyboardError::Ax(AxError::PermissionDenied, _)) => {
            eprintln!(
                "error: {}",
                format_ax_error(&AxError::PermissionDenied, None)
            );
            ExitCode::from(3)
        }
        Err(KeyboardError::Ax(AxError::StaleTarget, _)) => {
            eprintln!("error: {}", format_ax_error(&AxError::StaleTarget, None));
            ExitCode::from(4)
        }
        Err(KeyboardError::Ax(other, best_frame)) => {
            eprintln!("error: {}", format_ax_error(&other, best_frame));
            ExitCode::FAILURE
        }
    }
}

enum KeyboardError {
    Unregistered,
    /// AxError + best-known frame from the caller (if any was read before
    /// the failure). Transaction variants carry their own `actual` frame.
    Ax(AxError, Option<Rect>),
}

impl From<AxError> for KeyboardError {
    fn from(err: AxError) -> Self {
        Self::Ax(err, None)
    }
}

impl From<FixtureError> for KeyboardError {
    fn from(err: FixtureError) -> Self {
        match err {
            FixtureError::Unregistered => Self::Unregistered,
            FixtureError::Stale => Self::Ax(AxError::StaleTarget, None),
        }
    }
}

fn run(args: &[String]) -> Result<(), KeyboardError> {
    if args.len() < 2 {
        usage();
        return Err(KeyboardError::Ax(AxError::IllegalArgument, None));
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
                    let verified = adapter
                        .set_window_frame(&window, rect)
                        .map_err(|e| KeyboardError::Ax(e, Some(current)))?;
                    println!("ok moved {}", fmt_frame(&verified));
                    Ok(())
                }
                "resize" => {
                    let (w, h) = parse_two(args)?;
                    // Preserve the current position; resize is an explicit size set.
                    let current = adapter.window_frame(&window)?;
                    let rect = Rect::new(current.x, current.y, w, h);
                    let verified = adapter
                        .set_window_frame(&window, rect)
                        .map_err(|e| KeyboardError::Ax(e, Some(current)))?;
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
            Err(KeyboardError::Ax(AxError::IllegalArgument, None))
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

/// Format any AxError with its variant, rollback status, and best-known
/// frame. Transaction variants (`*WriteFailed`/`*ReadFailed`/
/// `VerificationFailed`) carry their own `actual` frame + rollback status.
/// Non-transaction variants report `rollback_restored=n/a` and the best-known
/// frame supplied by the caller (`<unavailable>` when none was read).
fn format_ax_error(err: &AxError, best_frame: Option<Rect>) -> String {
    let frame_str = || {
        best_frame
            .as_ref()
            .map(fmt_frame)
            .unwrap_or_else(|| "<unavailable>".into())
    };
    match err {
        AxError::PermissionDenied => format!(
            "permission denied (Accessibility not granted); rollback_restored=n/a; frame={}",
            frame_str()
        ),
        AxError::StaleTarget => format!(
            "stale target (process/window gone); rollback_restored=n/a; frame={}",
            frame_str()
        ),
        AxError::Unsupported => format!(
            "unsupported attribute/action; rollback_restored=n/a; frame={}",
            frame_str()
        ),
        AxError::IllegalArgument => {
            format!("illegal argument; rollback_restored=n/a; frame={}", frame_str())
        }
        AxError::NoWindows => {
            format!("no windows for target; rollback_restored=n/a; frame={}", frame_str())
        }
        AxError::InvalidGeometry => format!(
            "invalid geometry (non-finite or non-positive size); rollback_restored=n/a; frame={}",
            frame_str()
        ),
        AxError::PositionWriteFailed {
            source,
            rollback_restored,
            actual,
        } => format!(
            "position write rejected ({source}); rollback_restored={rollback_restored}; actual={}",
            actual.as_ref().map(fmt_frame).unwrap_or_else(|| "<unreadable>".into())
        ),
        AxError::SizeAfterPositionFailed {
            source,
            rollback_restored,
            actual,
        } => format!(
            "size write rejected after position applied ({source}); rollback_restored={rollback_restored}; actual={}",
            actual.as_ref().map(fmt_frame).unwrap_or_else(|| "<unreadable>".into())
        ),
        AxError::VerificationReadFailed {
            source,
            rollback_restored,
            actual,
        } => format!(
            "read-back failed after writes ({source}); rollback_restored={rollback_restored}; actual={}",
            actual.as_ref().map(fmt_frame).unwrap_or_else(|| "<unreadable>".into())
        ),
        AxError::VerificationFailed {
            expected,
            actual,
            rollback_restored,
        } => format!(
            "read-back mismatch (expected {}, actual {}); rollback_restored={rollback_restored}",
            fmt_frame(expected),
            fmt_frame(actual),
        ),
        AxError::Failure(code) => format!(
            "AX failure (code {code}); rollback_restored=n/a; frame={}",
            frame_str()
        ),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ax_error_reports_rollback_status_and_best_known_frame() {
        let before = Rect::new(100.0, 668.0, 420.0, 312.0);
        let best = Some(Rect::new(100.0, 668.0, 420.0, 312.0));

        // Transaction variants carry their own `actual` frame + rollback status.
        let err = AxError::PositionWriteFailed {
            source: Box::new(AxError::Failure(-25200)),
            rollback_restored: true,
            actual: Some(before),
        };
        let s = format_ax_error(&err, None);
        assert!(s.contains("rollback_restored=true"), "status: {s}");
        assert!(s.contains("x: 100"), "frame: {s}");

        let err = AxError::SizeAfterPositionFailed {
            source: Box::new(AxError::Failure(-25200)),
            rollback_restored: false,
            actual: Some(before),
        };
        let s = format_ax_error(&err, None);
        assert!(s.contains("rollback_restored=false"), "status: {s}");
        assert!(s.contains("x: 100"), "frame: {s}");

        let err = AxError::VerificationReadFailed {
            source: Box::new(AxError::Failure(-25204)),
            rollback_restored: false,
            actual: None,
        };
        let s = format_ax_error(&err, None);
        assert!(s.contains("read-back failed"), "variant: {s}");
        assert!(s.contains("rollback_restored=false"), "status: {s}");
        assert!(s.contains("<unreadable>"), "unreadable: {s}");

        let err = AxError::VerificationFailed {
            expected: Rect::new(120.0, 120.0, 420.0, 312.0),
            actual: Rect::new(999.0, 999.0, 420.0, 312.0),
            rollback_restored: false,
        };
        let s = format_ax_error(&err, None);
        assert!(s.contains("read-back mismatch"), "variant: {s}");
        assert!(s.contains("x: 999"), "actual frame: {s}");
        assert!(s.contains("rollback_restored=false"), "status: {s}");

        // Non-transaction variants: rollback_restored=n/a + best-known frame.
        let s = format_ax_error(&AxError::InvalidGeometry, best);
        assert!(s.contains("invalid geometry"), "variant: {s}");
        assert!(s.contains("rollback_restored=n/a"), "status: {s}");
        assert!(s.contains("x: 100"), "frame: {s}");

        let s = format_ax_error(&AxError::PermissionDenied, best);
        assert!(s.contains("permission denied"), "variant: {s}");
        assert!(s.contains("rollback_restored=n/a"), "status: {s}");
        assert!(s.contains("x: 100"), "frame: {s}");

        let s = format_ax_error(&AxError::StaleTarget, best);
        assert!(s.contains("stale target"), "variant: {s}");
        assert!(s.contains("rollback_restored=n/a"), "status: {s}");
        assert!(s.contains("x: 100"), "frame: {s}");

        let s = format_ax_error(&AxError::Unsupported, best);
        assert!(s.contains("unsupported"), "variant: {s}");
        assert!(s.contains("rollback_restored=n/a"), "status: {s}");

        let s = format_ax_error(&AxError::IllegalArgument, best);
        assert!(s.contains("illegal argument"), "variant: {s}");
        assert!(s.contains("rollback_restored=n/a"), "status: {s}");

        let s = format_ax_error(&AxError::NoWindows, best);
        assert!(s.contains("no windows"), "variant: {s}");
        assert!(s.contains("rollback_restored=n/a"), "status: {s}");

        let s = format_ax_error(&AxError::Failure(-25200), best);
        assert!(s.contains("AX failure"), "variant: {s}");
        assert!(s.contains("rollback_restored=n/a"), "status: {s}");
        assert!(s.contains("x: 100"), "frame: {s}");

        // No best frame available -> <unavailable>.
        let s = format_ax_error(&AxError::InvalidGeometry, None);
        assert!(s.contains("<unavailable>"), "unavailable: {s}");
    }
}
