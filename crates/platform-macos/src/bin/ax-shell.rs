//! Minimal macOS menu-bar shell (Gap 3).
//!
//! Runs the `NSStatusItem` shell in `hypr_rust_platform_macos::shell`. The menu
//! carries a status line, an explicit reflow command, pause/resume, a settings
//! summary, reset-to-defaults, an Accessibility status line, diagnostics and
//! quit. Auto-tiling stays off until the user picks a reflow command.
//!
//! Extra modes (no run loop):
//! - `--self-test` exercises the shell and returns an exit code: 0 pass,
//!   1 restore failure, 2 skipped run (pre-run config invalid/unsafe).
//! - `--panel-click-proof` dispatches Save through the real button action
//!   (performClick:) on the settings panel: 0 pass, 1 proof failure,
//!   2 skipped (a pre-existing config file is refused untouched).
//! - `--reset-defaults` writes the default config to `~/.hypr-rust/config.json`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--reset-defaults") {
        match hypr_rust_platform_macos::config::Config::default().save() {
            Ok(()) => println!("ok: wrote the default config"),
            Err(error) => {
                eprintln!("error: {error}");
                std::process::exit(1);
            }
        }
    } else if args.iter().any(|arg| arg == "--self-test") {
        // The self-test is evidence, not a demo: a failed restore or a
        // skipped run must return a failing exit code.
        let code = hypr_rust_platform_macos::shell::self_test();
        std::process::exit(code);
    } else if args.iter().any(|arg| arg == "--panel-click-proof") {
        // Same evidence-not-demo contract: fail closed on proof failure and
        // on the protective skip path (pre-existing config refused).
        let code = hypr_rust_platform_macos::shell::panel_click_proof();
        std::process::exit(code);
    } else {
        hypr_rust_platform_macos::shell::run();
    }
}
