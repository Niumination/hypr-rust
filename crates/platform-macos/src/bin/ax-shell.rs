//! Minimal macOS menu-bar shell (Gap 3).
//!
//! Runs the `NSStatusItem` shell in `hypr_rust_platform_macos::shell`. The menu
//! carries a status line, explicit reflow commands, pause/resume, an
//! Accessibility status line, diagnostics and quit. Auto-tiling stays off until
//! the user picks a reflow command.
//!
//! `--self-test` exercises the shell without a run loop: it creates the app,
//! status item and menu, reports the menu contents, then runs one reflow. That
//! is host evidence without a mouse click.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--self-test") {
        hypr_rust_platform_macos::shell::self_test();
    } else {
        hypr_rust_platform_macos::shell::run();
    }
}
