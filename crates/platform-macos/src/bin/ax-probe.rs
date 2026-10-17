//! CLI probe for native M1 tests. Operates only on an explicit pid given on
//! the command line (the self-owned HYPR-SPISE-TARGET spawned by the Swift
//! spike harness). Never enumerates user windows, never prints titles,
//! never mutates system UI.
//!
//! Usage:
//!   ax-probe frame <pid>
//!   ax-probe set-frame <pid> <x> <y> <width> <height>
//!   ax-probe raise <pid>
//!
//! Exit codes: 0 ok · 1 error · 3 permission denied · 4 stale target.

use std::process::ExitCode;

use hypr_rust_platform_macos::{AxError, MacAxAdapter};
use layout_core::Rect;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let result = run(&args);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(AxError::PermissionDenied) => {
            eprintln!("error: permission denied (Accessibility not granted)");
            ExitCode::from(3)
        }
        Err(AxError::StaleTarget) => {
            eprintln!("error: stale target (process/window gone)");
            ExitCode::from(4)
        }
        Err(other) => {
            eprintln!("error: {other}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), AxError> {
    if args.len() < 3 {
        eprintln!("usage: ax-probe <frame|set-frame|raise> <pid> [x y width height]");
        return Err(AxError::IllegalArgument);
    }
    let command = args[1].as_str();
    let pid: u32 = args[2].parse().map_err(|_| AxError::IllegalArgument)?;
    let adapter = MacAxAdapter::connect()?;
    let window = adapter.main_window(pid)?;

    match command {
        "frame" => {
            let rect = adapter.window_frame(&window)?;
            println!(
                "ok {{x: {}, y: {}, width: {}, height: {}}}",
                fmt(rect.x),
                fmt(rect.y),
                fmt(rect.width),
                fmt(rect.height)
            );
            Ok(())
        }
        "set-frame" => {
            if args.len() != 7 {
                eprintln!("usage: ax-probe set-frame <pid> <x> <y> <width> <height>");
                return Err(AxError::IllegalArgument);
            }
            let values: Vec<f64> = args[3..7]
                .iter()
                .map(|s| s.parse::<f64>())
                .collect::<Result<_, _>>()
                .map_err(|_| AxError::IllegalArgument)?;
            let rect = Rect::new(values[0], values[1], values[2], values[3]);
            adapter.set_window_frame(&window, rect)?;
            // Verify by reading back (acceptance: read-back within tolerance).
            let after = adapter.window_frame(&window)?;
            if (after.x - rect.x).abs() > 1.0
                || (after.y - rect.y).abs() > 1.0
                || (after.width - rect.width).abs() > 1.0
                || (after.height - rect.height).abs() > 1.0
            {
                println!("error: read-back mismatch: set {:?} got {:?}", rect, after);
                return Err(AxError::Failure(-25204));
            }
            println!(
                "ok {{x: {}, y: {}, width: {}, height: {}}}",
                fmt(after.x),
                fmt(after.y),
                fmt(after.width),
                fmt(after.height)
            );
            Ok(())
        }
        "raise" => {
            adapter.raise(&window)?;
            println!("ok raised");
            Ok(())
        }
        other => {
            eprintln!("unknown command: {other}");
            Err(AxError::IllegalArgument)
        }
    }
}

/// Print f64 without trailing noise; integers render as integers.
fn fmt(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}
